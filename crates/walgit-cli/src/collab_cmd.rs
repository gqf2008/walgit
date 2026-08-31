//! `walgit collab` — the D1 collaboration layer (`docs/D1_COLLAB_DESIGN.md`).
//!
//! Deterministic aggregation over `refs/collab/*` (§4.3): every client that
//! reads the same refs and verifies the same signatures computes the same
//! `thread` / `pr` / `merge_rule_eval` answer. The read commands run against a
//! local git checkout that has the collab refs (clone/fetch them), so no
//! server API is involved — this is the "anyone can verify locally" property.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

// ---- §4.2 entry schema -------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub version: u32,
    pub kind: String,
    pub id: String,
    pub actor: String,
    pub ts: i64,
    /// Previous entry's object id in the thread, or "" for the root.
    pub parent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<EntryRefs>,
    pub body: serde_json::Value,
    /// `ed25519:<base64>` over the canonical form of the entry without `sig`.
    #[serde(default)]
    pub sig: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EntryRefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

/// A parsed entry together with its object id and the principal whose inbox
/// holds it.
#[derive(Clone, Debug)]
pub struct EntryRef {
    pub oid: String,
    pub principal: String,
    pub entry: Entry,
}

// ---- §4.2 canonical form (the signed bytes) ----------------------------------

/// Recursive key-sorted, whitespace-free JSON: objects sorted by key bytes,
/// arrays in order, strings JSON-escaped, numbers as JSON numbers. This must
/// match the SDK's `canonicalize` (web/sdk/repos.ts) so cross-language
/// verifiers reproduce the exact signed input.
pub fn canonicalize(value: &serde_json::Value) -> String {
    let mut out = String::new();
    canonicalize_into(value, &mut out);
    out
}

fn canonicalize_into(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        serde_json::Value::String(s) => {
            // Infallible: serde_json always serializes a &str; default keeps
            // the canonical form total rather than panicking.
            out.push_str(&serde_json::to_string(s).unwrap_or_default());
        }
        serde_json::Value::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonicalize_into(v, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(o) => {
            out.push('{');
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_unstable();
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                canonicalize_into(&o[*k], out);
            }
            out.push('}');
        }
    }
}

/// The canonical bytes an entry's signature covers: the entry without `sig`.
pub fn entry_canonical(entry: &Entry) -> String {
    let mut unsigned = entry.clone();
    unsigned.sig.clear();
    let value = serde_json::to_value(unsigned).unwrap_or_default();
    canonicalize(&value)
}

// ---- verification ------------------------------------------------------------

/// Verify an entry against its principal's public key (base64) from
/// `refs/collab/meta/principals/<principal>`.
pub fn verify_entry(entry: &Entry, public_key_b64: &str) -> Result<(), String> {
    let sig_b64 = entry
        .sig
        .strip_prefix("ed25519:")
        .ok_or_else(|| "sig must be `ed25519:<base64>`".to_string())?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sig_b64)
        .map_err(|e| format!("bad sig base64: {e}"))?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|e| format!("bad signature: {e}"))?;
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64.trim())
        .map_err(|e| format!("bad public key base64: {e}"))?;
    let key_bytes: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| "public key must be 32 raw Ed25519 bytes".to_string())?;
    let vk = VerifyingKey::from_bytes(&key_bytes).map_err(|e| format!("bad public key: {e}"))?;
    vk.verify_strict(entry_canonical(entry).as_bytes(), &sig)
        .map_err(|e| format!("signature verification failed: {e}"))
}

/// Sign an entry's canonical form with a `SigningKey`; returns `ed25519:<base64>`.
/// Used by the write path (issue #14 slice ③: `walgit collab entry`); the
/// aggregation core only verifies, so this is currently exercised by tests.
#[allow(dead_code)]
pub fn sign_entry(entry: &mut Entry, key: &SigningKey) -> String {
    let canonical = entry_canonical(entry);
    let sig = key.sign(canonical.as_bytes());
    format!(
        "ed25519:{}",
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    )
}

// ---- §4.3 deterministic aggregation ------------------------------------------

/// One issue/thread: entries referencing the same `id`, topologically ordered
/// by the `parent` chain (deterministic: ts as the tie-break).
pub fn thread<'a>(entries: &[&'a EntryRef]) -> Vec<&'a EntryRef> {
    let by_oid: HashMap<&str, &EntryRef> = entries
        .iter()
        .map(|e| (e.oid.as_str(), *e))
        .collect();
    let mut emitted: HashMap<&str, bool> = HashMap::new();
    let mut out: Vec<&EntryRef> = Vec::new();
    // Deterministic first pass order: (ts, actor, oid).
    let mut pending: Vec<&EntryRef> = entries.to_vec();
    pending.sort_by(|a, b| {
        (a.entry.ts, a.entry.actor.as_str(), a.oid.as_str())
            .cmp(&(b.entry.ts, b.entry.actor.as_str(), b.oid.as_str()))
    });
    let mut guard = 0usize;
    while !pending.is_empty() && guard < pending.len() * 2 + 1 {
        guard += 1;
        let mut next: Vec<&EntryRef> = Vec::new();
        for e in pending {
            let ready = e.entry.parent.is_empty()
                || !by_oid.contains_key(e.entry.parent.as_str())
                || emitted.get(e.entry.parent.as_str()).copied().unwrap_or(false);
            if ready {
                emitted.insert(&e.oid, true);
                out.push(e);
            } else {
                next.push(e);
            }
        }
        pending = next;
    }
    out.extend(pending); // cycles or dangling parents: emit the rest deterministically
    out
}

/// The aggregated PR view for an `id` that has a `patch` entry.
#[derive(Serialize, Clone, Debug)]
pub struct PrView {
    pub id: String,
    pub base: Option<String>,
    pub head: Option<String>,
    pub status: String,
    pub reviews: Vec<Review>,
    /// Verified `approve` reviews by non-agent actors.
    pub human_approvals: Vec<Review>,
    pub unverified: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Review {
    pub actor: String,
    pub decision: String,
    pub ts: i64,
    pub oid: String,
}

fn is_agent(actor: &str) -> bool {
    actor.starts_with("svc-")
}

pub fn pr_view(
    entries: &[&EntryRef],
    principals: &HashMap<String, String>,
) -> PrView {
    let ordered = thread(entries);
    let mut base = None;
    let mut head = None;
    let mut status = "open".to_string();
    let mut reviews: Vec<Review> = Vec::new();
    let mut human_approvals: Vec<Review> = Vec::new();
    let mut unverified: Vec<String> = Vec::new();
    for r in &ordered {
        let e = &r.entry;
        if e.kind == "patch"
            && let Some(rs) = &e.refs
        {
            base = rs.base.clone().or(base);
            head = rs.head.clone().or(head);
        }
        let verified = principals
            .get(&e.actor)
            .is_some_and(|k| verify_entry(e, k).is_ok());
        if !verified {
            unverified.push(format!("{}@{}", e.actor, r.oid));
        }
        match e.kind.as_str() {
            "status" => {
                if let Some(st) = e.body.get("status").and_then(|v| v.as_str())
                    && (st == "merged" || st == "closed")
                {
                    status = st.to_string();
                }
            }
            "merge_result" => {
                if e.body
                    .get("merged")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    status = "merged".to_string();
                }
            }
            "review" => {
                let decision = e
                    .body
                    .get("decision")
                    .and_then(|v| v.as_str())
                    .unwrap_or("comment")
                    .to_string();
                let review = Review {
                    actor: e.actor.clone(),
                    decision: decision.clone(),
                    ts: e.ts,
                    oid: r.oid.clone(),
                };
                if decision == "approve" && verified {
                    human_approvals.push(review.clone());
                }
                reviews.push(review);
            }
            _ => {}
        }
    }
    PrView {
        id: ordered.first().map(|r| r.entry.id.clone()).unwrap_or_default(),
        base,
        head,
        status,
        reviews,
        human_approvals,
        unverified,
    }
}

// ---- merge rule evaluation ---------------------------------------------------

/// A minimal merge rule document (stored at `refs/collab/meta/rules` or given
/// on the CLI). D1 §6: merge rules are deterministic functions of the log.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MergeRules {
    /// Ref patterns whose merges require human approvals (e.g.
    /// `["refs/heads/main"]`). Empty = nothing protected.
    #[serde(default)]
    pub protect: Vec<String>,
    #[serde(default = "default_approvals")]
    pub require_human_approvals: usize,
}

fn default_approvals() -> usize {
    1
}

#[derive(Serialize, Clone, Debug)]
pub struct MergeEval {
    pub allowed: bool,
    pub reason: String,
    pub satisfied_by: Vec<String>,
}

/// Evaluate whether the PR may be merged: protected bases need at least
/// `require_human_approvals` verified approvals from non-agent actors.
pub fn merge_rule_eval(rules: &MergeRules, pr: &PrView) -> MergeEval {
    let protected = pr
        .base
        .as_deref()
        .is_some_and(|b| rules.protect.iter().any(|p| p == b || b.starts_with(p)));
    if !protected {
        return MergeEval {
            allowed: true,
            reason: "base is not protected".to_string(),
            satisfied_by: Vec::new(),
        };
    }
    let approvals: Vec<&Review> = pr
        .human_approvals
        .iter()
        .filter(|r| r.decision == "approve" && !is_agent(&r.actor))
        .collect();
    let satisfied_by: Vec<String> = approvals.iter().map(|r| r.actor.clone()).collect();
    if approvals.len() >= rules.require_human_approvals {
        MergeEval {
            allowed: true,
            reason: format!(
                "{} human approval(s) on protected base",
                approvals.len()
            ),
            satisfied_by,
        }
    } else {
        MergeEval {
            allowed: false,
            reason: format!(
                "protected base needs {} human approval(s), got {}",
                rules.require_human_approvals,
                approvals.len()
            ),
            satisfied_by,
        }
    }
}

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
            let filtered: Vec<&EntryRef> = entries
                .iter()
                .filter(|e| e.entry.id == id)
                .collect();
            if filtered.is_empty() {
                bail!("no entries for thread {id}");
            }
            let ordered = thread(&filtered);
            let out: Vec<serde_json::Value> = ordered
                .iter()
                .map(|r| {
                    let verified = principals
                        .get(&r.entry.actor)
                        .is_some_and(|k| verify_entry(&r.entry, k).is_ok());
                    serde_json::json!({ "oid": r.oid, "principal": r.principal, "verified": verified, "entry": r.entry })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        CollabAction::Pr { id, repo, rules } => {
            let reader = CollabReader::new(&repo);
            let (entries, principals) = reader.load()?;
            let filtered: Vec<&EntryRef> = entries
                .iter()
                .filter(|e| e.entry.id == id)
                .collect();
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
            let entry: Entry = serde_json::from_slice(&blob).with_context(|| {
                format!("parse entry at {name} ({oid})")
            })?;
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

    fn keypair() -> (ed25519_dalek::SigningKey, String) {
        // Deterministic test key (the CLI's write path takes the key from a
        // user file; generation is not part of the aggregation core).
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        (
            sk,
            base64::engine::general_purpose::STANDARD.encode(pk),
        )
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
        let v: serde_json::Value =
            serde_json::json!({"b": 1, "a": {"d": [1, 2], "c": "x"}});
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
        let e1 = entry("t", "comment", "alice", "missing", "a", 1, serde_json::json!({}));
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
        assert_eq!(pr.human_approvals.len(), 1, "only the verified approve counts");
        assert_eq!(pr.unverified.len(), 1, "tampered entry listed unverified");
    }
}
