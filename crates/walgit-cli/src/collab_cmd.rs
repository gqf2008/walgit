//! `walgit collab` — the D1 collaboration layer (`docs/D1_COLLAB_DESIGN.md`).
//!
//! Deterministic aggregation over `refs/collab/*` (§4.3): every client that
//! reads the same refs and verifies the same signatures computes the same
//! `thread` / `pr` / `merge_rule_eval` answer. The read commands run against a
//! local git checkout that has the collab refs (clone/fetch them), so no
//! server API is involved — this is the "anyone can verify locally" property.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use ed25519_dalek::SigningKey;
use std::fmt::Write as _;
use std::io::Write as _;
use walgit_wal::collab::{
    Entry, EntryRef, EntryRefs, MergeRules, Report, build_report, merge_rule_eval, pr_view,
    sign_entry, thread,
};

// ---- CLI commands --------------------------------------------------------------

#[derive(Subcommand)]
pub enum CollabAction {
    /// List distinct thread ids found in `refs/collab/inbox/*`.
    Ls {
        /// Local git checkout that has the collab refs.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// Print one thread (parent-ordered entries, per-entry verification) as JSON.
    Thread {
        id: String,
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// Print the aggregated PR view + merge rule evaluation as JSON.
    Pr {
        id: String,
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Merge-rules JSON file (`{"protect":["refs/heads/main"],...}`).
        #[arg(long)]
        rules: Option<PathBuf>,
    },
    /// Construct + sign + deliver a collab entry (§4.2). Writes the inbox ref
    /// locally; `--push <remote>` additionally pushes it to a walgit server.
    Entry {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Remote to push the ref to (omit for local-only writes).
        #[arg(long)]
        push: Option<String>,
        #[arg(long)]
        kind: String,
        /// Thread id (shared by every entry of the thread).
        #[arg(long)]
        id: String,
        /// Principal whose inbox receives the entry (refname-safe).
        #[arg(long)]
        actor: String,
        /// Previous entry's oid in the thread, or empty for the root.
        #[arg(long, default_value = "")]
        parent: String,
        /// Entry body as JSON.
        #[arg(long)]
        body: String,
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        head: Option<String>,
        /// Ed25519 signing key: 32 raw bytes as hex.
        #[arg(long)]
        key: PathBuf,
    },
    /// First-use registration of a principal's public key at
    /// `refs/collab/meta/principals/<principal>` (D1 §5).
    PrincipalRegister {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long)]
        principal: String,
        /// The signing key seed; the public key is derived from it.
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        push: Option<String>,
    },
    /// Revoke a principal's key: delete the registry ref (tombstone).
    PrincipalRevoke {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long)]
        principal: String,
        #[arg(long)]
        push: Option<String>,
    },
    /// Read-only observability dashboard (D1 §8): aggregate all collab state
    /// into a summary — threads, PR status, verification health, activity.
    Report {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Output format: text (default), markdown, html.
        #[arg(long, default_value = "text")]
        format: String,
        /// Merge-rules JSON file (same shape as `pr --rules`).
        #[arg(long)]
        rules: Option<PathBuf>,
    },
    /// Resident watcher: fetch `refs/collab/*` from a remote, report new or
    /// changed refs, and invoke `--exec` for each with the entry JSON on
    /// stdin (the agent's decision logic; walgit only does notify+sync).
    Watch {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Remote to fetch collab refs from.
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Seconds between passes (default 10).
        #[arg(long, default_value_t = 10)]
        interval: u64,
        /// Run a single pass and exit (tests/CI).
        #[arg(long)]
        once: bool,
        /// Command run for each new/changed ref (`sh -c`); entry JSON on stdin.
        #[arg(long)]
        exec: Option<String>,
        /// State file override (default `<gitdir>/collab-watch.json`).
        #[arg(long)]
        state: Option<PathBuf>,
    },
}

pub fn run(action: CollabAction) -> Result<()> {
    match action {
        CollabAction::Ls { repo } => {
            let reader = CollabReader::new(&repo);
            let (entries, _) = reader.load()?;
            let mut ids: Vec<&str> = entries.iter().map(|e| e.entry.id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            for id in ids {
                println!("{id}");
            }
        }
        CollabAction::Thread { id, repo } => {
            let reader = CollabReader::new(&repo);
            let (entries, principals) = reader.load()?;
            let filtered: Vec<&EntryRef> = entries.iter().filter(|e| e.entry.id == id).collect();
            if filtered.is_empty() {
                bail!("no entries for thread {id}");
            }
            let ordered = thread(&filtered);
            let out: Vec<serde_json::Value> = ordered
                .iter()
                .map(|r| {
                    let verified = r.is_verified(&principals);
                    serde_json::json!({ "oid": r.oid, "principal": r.principal, "verified": verified, "entry": r.entry })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        CollabAction::Entry {
            repo,
            push,
            kind,
            id,
            actor,
            parent,
            body,
            base,
            head,
            key,
        } => run_entry(&EntryArgs {
            repo,
            push,
            kind,
            id,
            actor,
            parent,
            body,
            base,
            head,
            key,
        })?,
        CollabAction::PrincipalRegister {
            repo,
            principal,
            key,
            push,
        } => run_principal_register(&repo, &principal, &key, push.as_deref())?,
        CollabAction::PrincipalRevoke {
            repo,
            principal,
            push,
        } => run_principal_revoke(&repo, &principal, push.as_deref())?,
        CollabAction::Report {
            repo,
            format,
            rules,
        } => run_report(&repo, &format, rules.as_deref())?,
        CollabAction::Watch {
            repo,
            remote,
            interval,
            once,
            exec,
            state,
        } => run_watch(
            &repo,
            &remote,
            interval,
            once,
            exec.as_deref(),
            state.as_deref(),
        )?,
        CollabAction::Pr { id, repo, rules } => {
            let reader = CollabReader::new(&repo);
            let (entries, principals) = reader.load()?;
            let filtered: Vec<&EntryRef> = entries.iter().filter(|e| e.entry.id == id).collect();
            let pr = pr_view(&filtered, &principals);
            let rules: MergeRules = match rules {
                Some(p) => serde_json::from_str(&std::fs::read_to_string(&p)?)?,
                None => MergeRules::default(),
            };
            let eval = merge_rule_eval(&rules, &pr);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "pr": pr, "merge": eval }))?
            );
        }
    }
    Ok(())
}

fn ref_segment(label: &str, s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.len() <= 255
        && !s.contains("..") // git forbids `..` inside a component
        && s != "."
        && s != ".."
        && !s.to_ascii_lowercase().ends_with(".lock")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-'))
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        bail!("collab.{label}: {s:?} is not a refname-safe segment ([A-Za-z0-9._@-]+)")
    }
}

fn read_signing_key(path: &std::path::Path) -> Result<SigningKey> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("read key {}", path.display()))?;
    let bytes = hex::decode(raw.trim())
        .with_context(|| format!("key {} must be 32 raw bytes as hex", path.display()))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("key {} must be 32 bytes", path.display()))?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn entry_uuid() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut b = [0u8; 16];
    rng.fill(&mut b);
    hex::encode(b)
}

/// Write a blob via `git hash-object -w --stdin`; returns the oid.
fn git_write_blob(repo: &std::path::Path, content: &str) -> Result<String> {
    let mut child = std::process::Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["hash-object", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("spawn git hash-object")?;
    child
        .stdin
        .take()
        .context("git hash-object stdin")?
        .write_all(content.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "git hash-object failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_update_ref(repo: &std::path::Path, name: &str, oid: Option<&str>) -> Result<()> {
    let args: Vec<&str> = if let Some(oid) = oid {
        vec!["update-ref", name, oid]
    } else {
        vec!["update-ref", "-d", name]
    };
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(&args)
        .output()
        .context("git update-ref")?;
    if !out.status.success() {
        bail!(
            "git update-ref {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn git_push(repo: &std::path::Path, remote: &str, name: &str) -> Result<()> {
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["push", remote, name])
        .output()
        .context("git push")?;
    if !out.status.success() {
        bail!(
            "git push {remote} {name} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

struct EntryArgs {
    repo: std::path::PathBuf,
    push: Option<String>,
    kind: String,
    id: String,
    actor: String,
    parent: String,
    body: String,
    base: Option<String>,
    head: Option<String>,
    key: std::path::PathBuf,
}

fn run_entry(args: &EntryArgs) -> Result<()> {
    ref_segment("entry.actor", &args.actor)?;
    let body: serde_json::Value = serde_json::from_str(&args.body)
        .with_context(|| format!("--body must be JSON: {}", args.body))?;
    let mut entry = Entry {
        version: 1,
        kind: args.kind.clone(),
        id: args.id.clone(),
        actor: args.actor.clone(),
        ts: chrono::Utc::now().timestamp(),
        parent: args.parent.clone(),
        refs: match (&args.base, &args.head) {
            (None, None) => None,
            (b, h) => Some(EntryRefs {
                base: b.clone(),
                head: h.clone(),
            }),
        },
        body,
        sig: String::new(),
    };
    let key = read_signing_key(&args.key)?;
    entry.sig = sign_entry(&mut entry, &key);
    let content = serde_json::to_string_pretty(&entry)?;
    let oid = git_write_blob(&args.repo, &content)?;
    let ref_name = format!("refs/collab/inbox/{}/{}", args.actor, entry_uuid());
    git_update_ref(&args.repo, &ref_name, Some(&oid))?;
    if let Some(remote) = &args.push {
        git_push(&args.repo, remote, &ref_name)?;
    }
    println!("{ref_name} {oid}");
    Ok(())
}

fn run_principal_register(
    repo: &Path,
    principal: &str,
    key_path: &Path,
    push: Option<&str>,
) -> Result<()> {
    ref_segment("principal", principal)?;
    let key = read_signing_key(key_path)?;
    let public_key =
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes());
    let content = serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "principal": principal,
        "public_key": public_key,
        "registered_at": chrono::Utc::now().timestamp(),
    }))?;
    let oid = git_write_blob(repo, &content)?;
    let ref_name = format!("refs/collab/meta/principals/{principal}");
    git_update_ref(repo, &ref_name, Some(&oid))?;
    if let Some(remote) = push {
        git_push(repo, remote, &ref_name)?;
    }
    println!("{ref_name} {oid}");
    Ok(())
}

fn run_principal_revoke(repo: &Path, principal: &str, push: Option<&str>) -> Result<()> {
    ref_segment("principal", principal)?;
    let ref_name = format!("refs/collab/meta/principals/{principal}");
    git_update_ref(repo, &ref_name, None)?;
    if let Some(remote) = push {
        git_push(repo, remote, &ref_name)?;
    }
    println!("{ref_name} revoked");
    Ok(())
}

fn render_report_text(r: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "collab report: {} threads, {} PRs, {}/{} entries verified\n",
        r.threads.len(),
        r.prs.len(),
        r.verified_entries,
        r.total_entries
    );
    let _ = writeln!(out, "threads");
    for t in &r.threads {
        let _ = writeln!(
            out,
            "  {}: {} entries ({} verified), kinds {}, last {}",
            t.id,
            t.entries,
            t.verified,
            t.kinds.join("/"),
            t.last_ts
        );
    }
    let _ = writeln!(out, "\nprs");
    for p in &r.prs {
        let _ = writeln!(
            out,
            "  {} [{}] approvals={} merge_allowed={} ({})",
            p.id, p.status, p.approvals, p.merge_allowed, p.merge_reason
        );
    }
    let _ = writeln!(out, "\nactivity");
    for (a, n) in &r.by_actor {
        let _ = writeln!(out, "  {a}: {n}");
    }
    out
}

fn render_report_markdown(r: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# collab report\n\n{} threads, {} PRs, **{}/{}** entries verified.\n",
        r.threads.len(),
        r.prs.len(),
        r.verified_entries,
        r.total_entries
    );
    let _ = writeln!(
        out,
        "## threads\n\n| id | entries | verified | kinds | last |\n|---|---|---|---|---|"
    );
    for t in &r.threads {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            t.id,
            t.entries,
            t.verified,
            t.kinds.join("/"),
            t.last_ts
        );
    }
    let _ = writeln!(
        out,
        "\n## PRs\n\n| id | status | approvals | merge |\n|---|---|---|---|"
    );
    for p in &r.prs {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} |",
            p.id, p.status, p.approvals, p.merge_allowed
        );
    }
    out
}

fn render_report_html(r: &Report) -> String {
    let mut rows = String::new();
    for t in &r.threads {
        let _ = writeln!(
            rows,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(&t.id),
            t.entries,
            t.verified,
            esc(&t.kinds.join("/")),
            t.last_ts
        );
    }
    let mut prs = String::new();
    for p in &r.prs {
        let _ = writeln!(
            prs,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(&p.id),
            esc(&p.status),
            p.approvals,
            p.merge_allowed
        );
    }
    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>walgit collab</title>\
<style>body{{font-family:system-ui;margin:2rem;color:#111}}table{{border-collapse:collapse}}td,th{{border:1px solid #ccc;padding:.3rem .6rem;text-align:left}}</style>\
</head><body><h1>collab report</h1>\
<p>{} threads, {} PRs, {}/{} entries verified</p>\
<h2>threads</h2><table><tr><th>id</th><th>entries</th><th>verified</th><th>kinds</th><th>last</th></tr>{}</table>\
<h2>PRs</h2><table><tr><th>id</th><th>status</th><th>approvals</th><th>merge</th></tr>{}</table>\
</body></html>",
        r.threads.len(),
        r.prs.len(),
        r.verified_entries,
        r.total_entries,
        rows,
        prs
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn run_report(repo: &Path, format: &str, rules_path: Option<&Path>) -> Result<()> {
    let reader = CollabReader::new(repo);
    let (entries, principals) = reader.load()?;
    let refs: Vec<&EntryRef> = entries.iter().collect();
    let rules: MergeRules = match rules_path {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
        None => MergeRules::default(),
    };
    let report = build_report(&refs, &principals, &rules);
    match format {
        "text" => print!("{}", render_report_text(&report)),
        "markdown" => print!("{}", render_report_markdown(&report)),
        "html" => print!("{}", render_report_html(&report)),
        other => bail!("unknown report format {other} (text|markdown|html)"),
    }
    Ok(())
}

// ---- watch: resident change detection + callback ------------------------------

/// Refs that are new or whose oid changed between two snapshots.
fn changed_refs(
    prev: &std::collections::HashMap<String, String>,
    cur: &std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = cur
        .iter()
        .filter(|(name, oid)| prev.get(*name) != Some(*oid))
        .map(|(n, o)| (n.clone(), o.clone()))
        .collect();
    out.sort();
    out
}

fn state_path(repo: &Path, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }
    let git_dir = std::process::Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .context("git rev-parse --absolute-git-dir")?;
    if !git_dir.status.success() {
        bail!("{} is not a git checkout", repo.display());
    }
    let dir = String::from_utf8_lossy(&git_dir.stdout).trim().to_string();
    Ok(PathBuf::from(dir).join("collab-watch.json"))
}

fn read_state(path: &Path) -> Result<std::collections::HashMap<String, String>> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Ok(std::collections::HashMap::new());
    };
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    let mut map = std::collections::HashMap::new();
    if let Some(o) = v.as_object() {
        for (k, val) in o {
            if let Some(oid) = val.as_str() {
                map.insert(k.clone(), oid.to_string());
            }
        }
    }
    Ok(map)
}

fn write_state(path: &Path, map: &std::collections::HashMap<String, String>) -> Result<()> {
    let mut obj = serde_json::Map::new();
    for (k, v) in map {
        obj.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&serde_json::Value::Object(obj))?,
    )
    .with_context(|| format!("write state {}", path.display()))?;
    Ok(())
}

fn git_fetch_collab(repo: &Path, remote: &str) -> Result<()> {
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["fetch", "-q", remote, "+refs/collab/*:refs/collab/*"])
        .output()
        .context("git fetch collab refs")?;
    if !out.status.success() {
        bail!(
            "git fetch {remote} refs/collab/* failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn refs_map(repo: &Path) -> Result<std::collections::HashMap<String, String>> {
    let reader = CollabReader::new(repo);
    let out = reader.git(&[
        "for-each-ref",
        "--format=%(refname) %(objectname)",
        "refs/collab",
    ])?;
    let mut map = std::collections::HashMap::new();
    for l in String::from_utf8_lossy(&out).lines() {
        let mut it = l.split_whitespace();
        if let (Some(n), Some(o)) = (it.next(), it.next()) {
            map.insert(n.to_string(), o.to_string());
        }
    }
    Ok(map)
}

/// One new/changed collab ref, described for the callback. The fields are
/// taken from the **parsed entry** — never re-parsed out of rendered text: the
/// blob is remote content, and a body containing a literal `\nkind=` line
/// must not be able to forge the signals an agent's `--exec` keys on.
struct RefEvent {
    kind: String,
    actor: String,
    thread: String,
    verified: bool,
    /// The raw blob text (the entry JSON on stdin).
    text: String,
}

fn describe_ref(repo: &Path, name: &str, oid: &str) -> Result<RefEvent> {
    let blob = CollabReader::new(repo).git(&["cat-file", "blob", oid])?;
    let principals = CollabReader::new(repo).principals()?;
    let text = String::from_utf8_lossy(&blob).to_string();
    if let Some(principal) = name.strip_prefix("refs/collab/meta/principals/") {
        return Ok(RefEvent {
            kind: "principal".into(),
            actor: principal.into(),
            thread: String::new(),
            verified: true,
            text,
        });
    }
    let entry: Entry = serde_json::from_str(&text).unwrap_or_else(|_| Entry {
        version: 0,
        kind: "unknown".into(),
        id: String::new(),
        actor: String::new(),
        ts: 0,
        parent: String::new(),
        refs: None,
        body: serde_json::Value::Null,
        sig: String::new(),
    });
    // The inbox path names the principal; verification includes the
    // inbox-consistency invariant (D1 §4.1, `EntryRef::is_verified`).
    let principal = name
        .strip_prefix("refs/collab/inbox/")
        .and_then(|p| p.rsplit_once('/'))
        .map(|(p, _)| p.to_string())
        .unwrap_or_default();
    let er = EntryRef {
        oid: oid.to_string(),
        principal,
        entry,
    };
    Ok(RefEvent {
        kind: er.entry.kind.clone(),
        actor: er.entry.actor.clone(),
        thread: er.entry.id.clone(),
        verified: er.is_verified(&principals),
        text,
    })
}

fn run_exec(cmd: &str, stdin: &str, env: &[(&str, &str)]) -> Result<()> {
    let mut out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .envs(env.iter().copied())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .context("spawn --exec")?;
    out.stdin
        .take()
        .context("--exec stdin")?
        .write_all(stdin.as_bytes())?;
    let status = out.wait_with_output().context("--exec")?.status;
    if !status.success() {
        bail!("--exec failed with {status}");
    }
    Ok(())
}

fn run_watch(
    repo: &Path,
    remote: &str,
    interval: u64,
    once: bool,
    exec: Option<&str>,
    state_override: Option<&Path>,
) -> Result<()> {
    let state_file = state_path(repo, state_override)?;
    loop {
        git_fetch_collab(repo, remote)?;
        let cur = refs_map(repo)?;
        let prev = read_state(&state_file)?;
        let changed = changed_refs(&prev, &cur);
        for (name, oid) in &changed {
            let ev = describe_ref(repo, name, oid)?;
            if let Some(cmd) = exec {
                let env: Vec<(&str, &str)> = vec![
                    ("WALGIT_COLLAB_REF", name.as_str()),
                    ("WALGIT_COLLAB_KIND", ev.kind.as_str()),
                    ("WALGIT_COLLAB_THREAD", ev.thread.as_str()),
                    ("WALGIT_COLLAB_ACTOR", ev.actor.as_str()),
                    (
                        "WALGIT_COLLAB_VERIFIED",
                        if ev.verified { "true" } else { "false" },
                    ),
                ];
                run_exec(cmd, &ev.text, &env)?;
            }
            println!("{name} {oid}");
        }
        write_state(&state_file, &cur)?;
        if once {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}

// ---- reading from a local git checkout --------------------------------------

pub struct CollabReader {
    repo: std::path::PathBuf,
}

impl CollabReader {
    pub fn new(repo: impl Into<std::path::PathBuf>) -> Self {
        Self { repo: repo.into() }
    }

    fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        let out = std::process::Command::new("git")
            .args(["-C"])
            .arg(&self.repo)
            .args(args)
            .output()
            .with_context(|| format!("run git {args:?} in {}", self.repo.display()))?;
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    }

    /// `refs/collab/inbox/<principal>/<uuid>` -> (refname, oid).
    fn inbox_refs(&self) -> Result<Vec<(String, String)>> {
        let out = self.git(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/collab/inbox",
        ])?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                match (it.next(), it.next()) {
                    (Some(r), Some(o)) => Some((r.to_string(), o.to_string())),
                    _ => None,
                }
            })
            .collect())
    }

    /// `refs/collab/meta/principals/<principal>` -> principal -> public key b64.
    fn principals(&self) -> Result<HashMap<String, String>> {
        let out = self.git(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/collab/meta/principals",
        ])?;
        let mut map = HashMap::new();
        for l in String::from_utf8_lossy(&out).lines() {
            let mut it = l.split_whitespace();
            let (Some(name), Some(oid)) = (it.next(), it.next()) else {
                continue;
            };
            let Some(principal) = name.strip_prefix("refs/collab/meta/principals/") else {
                continue;
            };
            let blob = self.git(&["cat-file", "blob", oid])?;
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&blob)
                && let Some(k) = v.get("public_key").and_then(|k| k.as_str())
            {
                map.insert(principal.to_string(), k.to_string());
            }
        }
        Ok(map)
    }

    /// Load every inbox entry with its oid/principal and the principals registry.
    pub fn load(&self) -> Result<(Vec<EntryRef>, HashMap<String, String>)> {
        let principals = self.principals()?;
        let mut entries = Vec::new();
        for (name, oid) in self.inbox_refs()? {
            let Some(principal) = name
                .strip_prefix("refs/collab/inbox/")
                .and_then(|p| p.rsplit_once('/'))
                .map(|(p, _)| p.to_string())
            else {
                continue;
            };
            let blob = self.git(&["cat-file", "blob", &oid])?;
            let entry: Entry = serde_json::from_slice(&blob)
                .with_context(|| format!("parse entry at {name} ({oid})"))?;
            entries.push(EntryRef {
                oid,
                principal,
                entry,
            });
        }
        Ok((entries, principals))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use walgit_wal::collab::{Review, canonicalize, verify_entry};

    fn keypair() -> (ed25519_dalek::SigningKey, String) {
        // Deterministic test key (the CLI's write path takes the key from a
        // user file; generation is not part of the aggregation core).
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        (sk, base64::engine::general_purpose::STANDARD.encode(pk))
    }

    fn entry(
        id: &str,
        kind: &str,
        actor: &str,
        parent: &str,
        oid: &str,
        ts: i64,
        body: serde_json::Value,
    ) -> EntryRef {
        EntryRef {
            oid: oid.into(),
            principal: actor.into(),
            entry: Entry {
                version: 1,
                kind: kind.into(),
                id: id.into(),
                actor: actor.into(),
                ts,
                parent: parent.into(),
                refs: None,
                body,
                sig: String::new(),
            },
        }
    }

    #[test]
    fn canonicalize_is_sorted_and_compact() {
        let v: serde_json::Value = serde_json::json!({"b": 1, "a": {"d": [1, 2], "c": "x"}});
        assert_eq!(canonicalize(&v), r#"{"a":{"c":"x","d":[1,2]},"b":1}"#);
    }

    #[test]
    fn sign_verify_roundtrip_and_tamper() {
        let (sk, pk) = keypair();
        let mut e = entry(
            "1",
            "issue",
            "alice",
            "",
            "abc",
            1,
            serde_json::json!({"title": "t"}),
        );
        e.entry.sig = sign_entry(&mut e.entry, &sk);
        assert!(verify_entry(&e.entry, &pk).is_ok());
        e.entry.body = serde_json::json!({"title": "tampered"});
        assert!(verify_entry(&e.entry, &pk).is_err());
    }

    #[test]
    fn thread_orders_by_parent_chain() {
        let e1 = entry("t", "comment", "alice", "", "a", 1, serde_json::json!({}));
        let mut e2 = entry("t", "comment", "bob", "a", "b", 10, serde_json::json!({}));
        e2.entry.refs = None;
        let refs = vec![&e2, &e1]; // out of order input
        let ordered = thread(&refs);
        assert_eq!(ordered[0].oid, "a");
        assert_eq!(ordered[1].oid, "b");
        assert_eq!(ordered.len(), 2);
    }

    #[test]
    fn thread_deterministic_with_dangling_parent() {
        let e1 = entry(
            "t",
            "comment",
            "alice",
            "missing",
            "a",
            1,
            serde_json::json!({}),
        );
        let e2 = entry("t", "comment", "bob", "", "b", 2, serde_json::json!({}));
        let refs = vec![&e1, &e2];
        let ordered = thread(&refs);
        // Dangling parent is treated as a root; order is (ts, actor, oid).
        assert_eq!(ordered[0].oid, "a");
        assert_eq!(ordered[1].oid, "b");
    }

    #[test]
    fn merge_rule_counts_only_human_approvals_on_protected_base() {
        let rules = MergeRules {
            protect: vec!["refs/heads/main".into()],
            require_human_approvals: 1,
        };
        let mut pr = pr_view(&[], &HashMap::new());
        pr.base = Some("refs/heads/main".into());
        assert!(!merge_rule_eval(&rules, &pr).allowed, "no approvals");

        pr.human_approvals.push(Review {
            actor: "alice".into(),
            decision: "approve".into(),
            ts: 1,
            oid: "x".into(),
        });
        let eval = merge_rule_eval(&rules, &pr);
        assert!(eval.allowed, "{eval:?}");
        assert_eq!(eval.satisfied_by, vec!["alice"]);

        let mut agent_only = pr.clone();
        agent_only.human_approvals = vec![Review {
            actor: "svc-reviewer".into(),
            decision: "approve".into(),
            ts: 1,
            oid: "y".into(),
        }];
        assert!(
            !merge_rule_eval(&rules, &agent_only).allowed,
            "agent approval does not count"
        );
    }

    #[test]
    fn report_is_deterministic_and_counts_verification() {
        let (sk, pk) = keypair();
        let mut principals = HashMap::new();
        principals.insert("alice".to_string(), pk);

        let mut issue = entry(
            "pr1",
            "issue",
            "alice",
            "",
            "a",
            1,
            serde_json::json!({"title": "t"}),
        );
        issue.entry.sig = sign_entry(&mut issue.entry, &sk);
        let unsigned = entry("pr1", "comment", "bob", "a", "b", 2, serde_json::json!({}));
        let mut patch = entry("pr1", "patch", "alice", "b", "c", 3, serde_json::json!({}));
        patch.entry.refs = Some(EntryRefs {
            base: Some("refs/heads/main".into()),
            head: Some("refs/heads/topic".into()),
        });
        patch.entry.sig = sign_entry(&mut patch.entry, &sk);
        let mut review = entry(
            "pr1",
            "review",
            "alice",
            "c",
            "d",
            4,
            serde_json::json!({"decision": "approve"}),
        );
        review.entry.sig = sign_entry(&mut review.entry, &sk);

        let refs = vec![&issue, &unsigned, &patch, &review];
        let rules = MergeRules {
            protect: vec!["refs/heads/main".into()],
            require_human_approvals: 1,
        };
        let r1 = build_report(&refs, &principals, &rules);
        assert_eq!(r1.threads.len(), 1);
        assert_eq!(r1.threads[0].id, "pr1");
        assert_eq!(r1.threads[0].entries, 4);
        assert_eq!(
            r1.threads[0].verified, 3,
            "bob's unsigned entry not verified"
        );
        assert_eq!(r1.total_entries, 4);
        assert_eq!(r1.verified_entries, 3);
        assert_eq!(r1.unverified_entries, 1);
        assert_eq!(r1.missing_principals, 1, "bob has no key");
        assert_eq!(r1.prs.len(), 1);
        assert_eq!(r1.prs[0].approvals, 1);
        assert!(r1.prs[0].merge_allowed);
        assert_eq!(
            r1.by_actor,
            vec![("alice".to_string(), 3), ("bob".to_string(), 1)]
        );

        // Determinism: identical input -> identical text render.
        let r2 = build_report(&refs, &principals, &rules);
        assert_eq!(render_report_text(&r1), render_report_text(&r2));
        assert_eq!(render_report_markdown(&r1), render_report_markdown(&r2));
        assert_eq!(render_report_html(&r1), render_report_html(&r2));
        assert!(render_report_html(&r1).contains("<!doctype html>"));
        assert!(render_report_html(&r1).contains("</html>"));
    }

    #[test]
    fn changed_refs_reports_new_and_updated_only() {
        let mut prev = std::collections::HashMap::new();
        prev.insert("refs/collab/inbox/a/1".to_string(), "aaa".to_string());
        prev.insert("refs/collab/inbox/a/2".to_string(), "bbb".to_string());
        let mut cur = std::collections::HashMap::new();
        cur.insert("refs/collab/inbox/a/1".to_string(), "aaa".to_string()); // unchanged
        cur.insert("refs/collab/inbox/a/2".to_string(), "ccc".to_string()); // updated
        cur.insert("refs/collab/inbox/b/3".to_string(), "ddd".to_string()); // new
        let changed = changed_refs(&prev, &cur);
        assert_eq!(
            changed,
            vec![
                ("refs/collab/inbox/a/2".to_string(), "ccc".to_string()),
                ("refs/collab/inbox/b/3".to_string(), "ddd".to_string()),
            ]
        );
    }

    #[test]
    fn pr_view_marks_unverified_approvals() {
        let (sk, pk) = keypair();
        let mut e = entry(
            "pr1",
            "review",
            "alice",
            "",
            "r1",
            1,
            serde_json::json!({"decision": "approve"}),
        );
        e.entry.sig = sign_entry(&mut e.entry, &sk);
        let good = e.clone();
        let mut tampered = e.clone();
        tampered.entry.body = serde_json::json!({"decision": "request_changes"});

        let mut principals = HashMap::new();
        principals.insert("alice".to_string(), pk);
        let refs = vec![&good, &tampered];
        let pr = pr_view(&refs, &principals);
        assert_eq!(
            pr.human_approvals.len(),
            1,
            "only the verified approve counts"
        );
        assert_eq!(pr.unverified.len(), 1, "tampered entry listed unverified");
    }

    #[test]
    fn an_entry_in_the_wrong_inbox_is_not_verified() {
        let (sk, pk) = keypair();
        let mut e = entry(
            "pr1",
            "review",
            "alice", // the JSON names alice
            "",
            "r1",
            1,
            serde_json::json!({"decision": "approve"}),
        );
        e.entry.sig = sign_entry(&mut e.entry, &sk);
        let mut principals = HashMap::new();
        principals.insert("alice".to_string(), pk);

        // Same actor, own inbox: verified.
        let mut own = e.clone();
        own.principal = "alice".into();
        assert!(own.is_verified(&principals));

        // The same signed bytes found in someone else's inbox: the signature is
        // alice's, but the inbox model (D1 §4.1) shards by principal — an entry
        // in bob's inbox naming alice as actor does not count.
        let mut smuggled = e.clone();
        smuggled.principal = "bob".into();
        assert!(!smuggled.is_verified(&principals));

        let refs = vec![&smuggled];
        let pr = pr_view(&refs, &principals);
        assert!(
            pr.human_approvals.is_empty(),
            "an approval smuggled into another inbox does not count"
        );
        assert_eq!(pr.unverified.len(), 1);
    }
}
