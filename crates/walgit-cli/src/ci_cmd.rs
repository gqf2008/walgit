//! `walgit ci` — the decentralized CI protocol (`docs/D1_CI_PROTOCOL.md`).
//!
//! walgit runs no CI (principle X): the server is only the fact source (bucket)
//! and the event source (ref facts). A **runner** is a credentialed client that
//! subscribes to ref updates, claims a run with a signed `ci_claim` entry,
//! executes the task declared in the tested commit's `.walgit/ci.toml`, and
//! publishes a signed `ci_result` entry back into its collab inbox. Everything
//! this module writes, the aggregation core (`walgit-wal/src/ci.rs`) can
//! re-derive and verify offline — the protocol has no server-side logic.

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Deserialize;
use std::path::PathBuf;

// ---- schema limits (docs/D1_CI_PROTOCOL.md §3.1, normative bounds) -------------

/// V9: the file itself.
const FILE_MAX_BYTES: usize = 64 * 1024;
/// V4: the command string.
const COMMAND_MAX_BYTES: usize = 4096;
/// V3: tasks per file.
const TASKS_MAX: usize = 64;
/// V3: task name length.
const NAME_MAX: usize = 64;
/// V5: ref patterns per task, and per-pattern length.
const REFS_PER_TASK_MAX: usize = 64;
const REF_PATTERN_MAX: usize = 255;
/// V6: timeout / `claim_ttl` bounds, in seconds.
const TIMEOUT_MIN_SECS: u64 = 1;
const TIMEOUT_MAX_SECS: u64 = 24 * 3600;
/// V7: retry bound.
const ATTEMPTS_MAX: u32 = 10;
/// V8: `env_allow` entries.
const ENV_ALLOW_MAX: usize = 64;

/// Pipeline-level defaults (§3), applied where a task omits the field.
const DEFAULT_REFS: [&str; 1] = ["refs/heads/*"];
const DEFAULT_TIMEOUT_SECS: u64 = 600;
const DEFAULT_CLAIM_TTL_SECS: u64 = 300;
const DEFAULT_ATTEMPTS: u32 = 1;

#[derive(Subcommand)]
pub enum CiAction {
    /// Parse and validate a `.walgit/ci.toml` (`docs/D1_CI_PROTOCOL.md` §3.1).
    Validate {
        /// Repository (or any directory) holding the file.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Explicit file path (default `<repo>/.walgit/ci.toml`).
        #[arg(long)]
        file: Option<PathBuf>,
    },
}

pub fn run(action: CiAction) -> Result<()> {
    match action {
        CiAction::Validate { repo, file } => {
            let path = file.unwrap_or_else(|| repo.join(".walgit").join("ci.toml"));
            let raw = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            let resolved =
                parse_and_validate(&raw).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            println!(
                "{}: ok — claim_ttl {}s, {} task(s)",
                path.display(),
                resolved.claim_ttl_secs,
                resolved.tasks.len()
            );
            for t in &resolved.tasks {
                println!(
                    "  {} refs [{}] timeout {}s attempts {} env_allow [{}]\n    {}",
                    t.name,
                    t.refs.join(", "),
                    t.timeout_secs,
                    t.max_attempts,
                    t.env_allow.join(", "),
                    t.command
                );
            }
            Ok(())
        }
    }
}

// ---- .walgit/ci.toml schema (docs/D1_CI_PROTOCOL.md §3) ------------------------

/// The raw file. Unknown keys are a validation failure (V1) — a typo in a
/// task declaration must not silently disable a check.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CiConfig {
    version: u32,
    #[serde(default)]
    claim_ttl: Option<String>,
    #[serde(default)]
    timeout: Option<String>,
    #[serde(default)]
    max_attempts: Option<u32>,
    #[serde(default)]
    task: Vec<CiTask>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CiTask {
    name: String,
    #[serde(default)]
    refs: Option<Vec<String>>,
    command: String,
    #[serde(default)]
    timeout: Option<String>,
    #[serde(default)]
    max_attempts: Option<u32>,
    #[serde(default)]
    env_allow: Option<Vec<String>>,
}

/// A validated task with pipeline defaults applied — what the runner executes.
#[derive(Clone, Debug)]
pub struct CiResolvedTask {
    pub name: String,
    pub refs: Vec<String>,
    pub command: String,
    pub timeout_secs: u64,
    pub max_attempts: u32,
    pub env_allow: Vec<String>,
}

/// A validated `.walgit/ci.toml`.
#[derive(Clone, Debug)]
pub struct CiResolved {
    pub claim_ttl_secs: u64,
    pub tasks: Vec<CiResolvedTask>,
}

impl CiResolved {
    /// The tasks whose `refs` patterns match `ref_name`, in declaration order.
    // `#[allow(dead_code)]`: consumed by the runner (`ci run`, batch #31 unit 3);
    // it lives with the schema so the trigger-matching contract is tested here.
    #[allow(dead_code)]
    pub fn matching(&self, ref_name: &str) -> Vec<&CiResolvedTask> {
        self.tasks
            .iter()
            .filter(|t| t.refs.iter().any(|p| glob_match(p, ref_name)))
            .collect()
    }
}

/// Parse + validate (§3.1 V1–V9). `Err` carries every violation found.
pub fn parse_and_validate(raw: &[u8]) -> Result<CiResolved, String> {
    let mut errors = Vec::new();
    if raw.len() > FILE_MAX_BYTES {
        return Err(format!(
            "V9: file is {} bytes, limit {FILE_MAX_BYTES}",
            raw.len()
        ));
    }
    let text = match std::str::from_utf8(raw) {
        Ok(t) => t,
        Err(e) => return Err(format!("file is not UTF-8: {e}")),
    };
    let cfg: CiConfig = match toml::from_str(text) {
        Ok(c) => c,
        Err(e) => return Err(format!("V1: {e}")),
    };
    if cfg.version != 1 {
        errors.push(format!("V2: version must be 1, got {}", cfg.version));
    }
    if cfg.task.is_empty() {
        errors.push("V3: at least one [[task]] is required".to_string());
    }
    if cfg.task.len() > TASKS_MAX {
        errors.push(format!("V3: {} tasks, limit {TASKS_MAX}", cfg.task.len()));
    }
    let claim_ttl_secs = match cfg.claim_ttl.as_deref() {
        None => DEFAULT_CLAIM_TTL_SECS,
        Some(s) => match parse_duration_secs(s) {
            Ok(secs) if (TIMEOUT_MIN_SECS..=TIMEOUT_MAX_SECS).contains(&secs) => secs,
            Ok(secs) => {
                errors.push(format!(
                    "V6: claim_ttl {s:?} = {secs}s outside [{TIMEOUT_MIN_SECS}, {TIMEOUT_MAX_SECS}]"
                ));
                DEFAULT_CLAIM_TTL_SECS
            }
            Err(e) => {
                errors.push(format!("V6: claim_ttl {s:?}: {e}"));
                DEFAULT_CLAIM_TTL_SECS
            }
        },
    };
    // Pipeline-level defaults: a task that omits a field inherits these (§3).
    let pipeline_timeout_secs = match cfg.timeout.as_deref() {
        None => DEFAULT_TIMEOUT_SECS,
        Some(s) => match parse_duration_secs(s) {
            Ok(secs) if (TIMEOUT_MIN_SECS..=TIMEOUT_MAX_SECS).contains(&secs) => secs,
            Ok(_) | Err(_) => {
                errors.push(format!("V6: pipeline timeout {s:?} is not a valid duration in [{TIMEOUT_MIN_SECS}, {TIMEOUT_MAX_SECS}]"));
                DEFAULT_TIMEOUT_SECS
            }
        },
    };
    let pipeline_max_attempts = match cfg.max_attempts {
        None => DEFAULT_ATTEMPTS,
        Some(n) => {
            check(
                &mut errors,
                (1..=ATTEMPTS_MAX).contains(&n),
                format!("V7: pipeline max_attempts {n} outside [1, {ATTEMPTS_MAX}]"),
            );
            n
        }
    };
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut tasks = Vec::new();
    for t in &cfg.task {
        let name = &t.name;
        check(
            &mut errors,
            name.len() <= NAME_MAX
                && !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
            format!("V3: task name {name:?} must match [A-Za-z0-9._-]{{1,{NAME_MAX}}}"),
        );
        check(
            &mut errors,
            seen.insert(name.as_str()),
            format!("V3: duplicate task name {name:?}"),
        );
        check(
            &mut errors,
            !t.command.is_empty() && t.command.len() <= COMMAND_MAX_BYTES,
            format!(
                "V4: task {name:?} command must be 1..{COMMAND_MAX_BYTES} bytes, got {}",
                t.command.len()
            ),
        );
        let refs = match &t.refs {
            None => DEFAULT_REFS.iter().map(|s| (*s).to_string()).collect(),
            Some(v) => v.clone(),
        };
        check(
            &mut errors,
            !refs.is_empty() && refs.len() <= REFS_PER_TASK_MAX,
            format!("V5: task {name:?} refs must hold 1..{REFS_PER_TASK_MAX} patterns"),
        );
        for r in &refs {
            check(
                &mut errors,
                !r.is_empty() && r.len() <= REF_PATTERN_MAX && r.starts_with("refs/"),
                format!(
                    "V5: task {name:?} ref pattern {r:?} must start with `refs/` and be ≤ {REF_PATTERN_MAX} bytes"
                ),
            );
        }
        let timeout_secs = match t.timeout.as_deref() {
            None => pipeline_timeout_secs,
            Some(s) => match parse_duration_secs(s) {
                Ok(secs) if (TIMEOUT_MIN_SECS..=TIMEOUT_MAX_SECS).contains(&secs) => secs,
                Ok(secs) => {
                    errors.push(format!(
                        "V6: task {name:?} timeout {s:?} = {secs}s outside [{TIMEOUT_MIN_SECS}, {TIMEOUT_MAX_SECS}]"
                    ));
                    pipeline_timeout_secs
                }
                Err(e) => {
                    errors.push(format!("V6: task {name:?} timeout {s:?}: {e}"));
                    pipeline_timeout_secs
                }
            },
        };
        let max_attempts = t.max_attempts.unwrap_or(pipeline_max_attempts);
        check(
            &mut errors,
            (1..=ATTEMPTS_MAX).contains(&max_attempts),
            format!("V7: task {name:?} max_attempts {max_attempts} outside [1, {ATTEMPTS_MAX}]"),
        );
        let env_allow = t.env_allow.clone().unwrap_or_default();
        check(
            &mut errors,
            env_allow.len() <= ENV_ALLOW_MAX,
            format!(
                "V8: task {name:?} env_allow has {} entries, limit {ENV_ALLOW_MAX}",
                env_allow.len()
            ),
        );
        for env_name in &env_allow {
            check(
                &mut errors,
                valid_env_name(env_name) && !env_name.starts_with("WALGIT_CI_"),
                format!(
                    "V8: task {name:?} env_allow {env_name:?} is not a valid env name (or reserves the WALGIT_CI_* namespace, which the runner injects)"
                ),
            );
        }
        tasks.push(CiResolvedTask {
            name: name.clone(),
            refs,
            command: t.command.clone(),
            timeout_secs,
            max_attempts,
            env_allow,
        });
    }
    if errors.is_empty() {
        Ok(CiResolved {
            claim_ttl_secs,
            tasks,
        })
    } else {
        Err(errors.join("\n"))
    }
}

/// Record one violation of §3.1.
fn check(errors: &mut Vec<String>, ok: bool, msg: String) {
    if !ok {
        errors.push(msg);
    }
}

/// `humantime` duration (`"30s"`, `"10m"`, `"1h"`) → whole seconds.
fn parse_duration_secs(s: &str) -> Result<u64, String> {
    humantime::parse_duration(s)
        .map_err(|e| format!("not a duration: {e}"))
        .map(|d| d.as_secs())
}

/// `[A-Za-z_][A-Za-z0-9_]*`.
fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Glob over ref names (§3.2): `*` matches any run of characters **including
/// `/`**, `?` one character, everything else literal; the whole name must
/// match. Iterative greedy backtracking, no allocations per candidate.
// `#[allow(dead_code)]`: consumed by the runner (`ci run`, batch #31 unit 3);
// it lives with the schema so the glob contract is tested here.
#[allow(dead_code)]
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let mut star = usize::MAX;
    let mut mark = 0usize;
    while let Some(&nc) = n.get(ni) {
        let pc = p.get(pi).copied();
        if pc.is_some_and(|c| c == '?' || c == nc) {
            pi += 1;
            ni += 1;
        } else if pc == Some('*') {
            star = pi;
            mark = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while p.get(pi) == Some(&'*') {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> CiResolved {
        parse_and_validate(raw.as_bytes()).unwrap_or_else(|e| panic!("should validate: {e}"))
    }

    fn errs(raw: &str) -> String {
        parse_and_validate(raw.as_bytes()).expect_err("should fail validation")
    }

    const MINIMAL: &str = r#"
version = 1
[[task]]
name = "test"
command = "cargo test"
"#;

    #[test]
    fn minimal_valid_with_defaults() {
        let r = ok(MINIMAL);
        assert_eq!(r.claim_ttl_secs, DEFAULT_CLAIM_TTL_SECS);
        assert_eq!(r.tasks.len(), 1);
        let t = &r.tasks[0];
        assert_eq!(t.refs, vec!["refs/heads/*"], "default refs");
        assert_eq!(t.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert_eq!(t.max_attempts, 1);
        assert!(t.env_allow.is_empty());
        assert_eq!(r.matching("refs/heads/main").len(), 1);
        assert!(
            r.matching("refs/tags/v1").is_empty(),
            "default does not match tags"
        );
    }

    #[test]
    fn task_level_overrides_and_star_crosses_slashes() {
        let r = ok(r#"
version = 1
claim_ttl = "30s"
[[task]]
name = "tag.build"
refs = ["refs/tags/v*"]
command = "make release"
timeout = "1h"
max_attempts = 3
env_allow = ["RUSTFLAGS"]
"#);
        assert_eq!(r.claim_ttl_secs, 30);
        assert_eq!(r.tasks[0].timeout_secs, 3600);
        assert_eq!(r.tasks[0].max_attempts, 3);
        assert_eq!(r.tasks[0].env_allow, vec!["RUSTFLAGS"]);
        assert_eq!(r.matching("refs/tags/v1.2").len(), 1, "`*` crosses `/`");
        assert!(r.matching("refs/heads/main").is_empty());
    }

    #[test]
    fn unknown_keys_are_rejected_top_level_and_per_task() {
        let e = errs(
            r#"
version = 1
concurrency = 4
[[task]]
name = "a"
command = "true"
"#,
        );
        assert!(e.contains("V1"), "{e}");
        let e = errs(
            r#"
version = 1
[[task]]
name = "a"
command = "true"
secrets = ["x"]
"#,
        );
        assert!(e.contains("V1"), "{e}");
    }

    #[test]
    fn validation_matrix() {
        assert!(errs("version = 2\n[[task]]\nname=\"a\"\ncommand=\"true\"\n").contains("V2"));
        assert!(errs("version = 1\n").contains("V3"));
        assert!(errs(
            "version = 1\n[[task]]\nname=\"a\"\ncommand=\"x\"\n[[task]]\nname=\"a\"\ncommand=\"y\"\n"
        )
        .contains("duplicate task name"));
        assert!(errs("version = 1\n[[task]]\nname=\"a b\"\ncommand=\"true\"\n").contains("V3"));
        assert!(errs("version = 1\n[[task]]\nname=\"a\"\ncommand=\"\"\n").contains("V4"));
        assert!(
            errs("version = 1\n[[task]]\nname=\"a\"\ncommand=\"true\"\nrefs=[\"heads/x\"]\n")
                .contains("V5")
        );
        assert!(
            errs("version = 1\n[[task]]\nname=\"a\"\ncommand=\"true\"\ntimeout=\"0s\"\n")
                .contains("V6")
        );
        assert!(
            errs("version = 1\n[[task]]\nname=\"a\"\ncommand=\"true\"\ntimeout=\"soon\"\n")
                .contains("V6")
        );
        assert!(
            errs("version = 1\nclaim_ttl = \"48h\"\n[[task]]\nname=\"a\"\ncommand=\"true\"\n")
                .contains("V6")
        );
        assert!(
            errs("version = 1\n[[task]]\nname=\"a\"\ncommand=\"true\"\nmax_attempts=0\n")
                .contains("V7")
        );
        assert!(
            errs("version = 1\n[[task]]\nname=\"a\"\ncommand=\"true\"\nmax_attempts=11\n")
                .contains("V7")
        );
        assert!(
            errs("version = 1\n[[task]]\nname=\"a\"\ncommand=\"true\"\nenv_allow=[\"1BAD\"]\n")
                .contains("V8")
        );
        assert!(errs(
            "version = 1\n[[task]]\nname=\"a\"\ncommand=\"true\"\nenv_allow=[\"WALGIT_CI_TASK\"]\n"
        )
        .contains("V8"));
        assert!(
            errs(
                "version = 1\n[[task]]\nname=\"a\"\ncommand=\"true\"\nenv_allow=[\"HAS SPACE\"]\n"
            )
            .contains("V8")
        );
    }

    #[test]
    fn pipeline_defaults_are_inherited_by_omitting_tasks() {
        let r = ok(r#"
version = 1
claim_ttl = "30s"
timeout = "2m"
max_attempts = 4
[[task]]
name = "inherits"
command = "true"
[[task]]
name = "overrides"
command = "true"
timeout = "3m"
max_attempts = 2
"#);
        assert_eq!(r.claim_ttl_secs, 30);
        let inherits = &r.tasks[0];
        assert_eq!(inherits.timeout_secs, 120, "pipeline timeout inherited");
        assert_eq!(inherits.max_attempts, 4, "pipeline attempts inherited");
        let overrides = &r.tasks[1];
        assert_eq!(overrides.timeout_secs, 180);
        assert_eq!(overrides.max_attempts, 2);
        // A bad pipeline-level value is still a validation failure.
        assert!(
            errs("version = 1\nmax_attempts = 0\n[[task]]\nname=\"a\"\ncommand=\"t\"\n")
                .contains("V7")
        );
    }

    #[test]
    fn glob_semantics() {
        assert!(
            glob_match("refs/heads/*", "refs/heads/a/b"),
            "`*` crosses `/`"
        );
        assert!(glob_match("refs/tags/v*", "refs/tags/v1.0.0"));
        assert!(
            !glob_match("refs/heads/main", "refs/heads/mainx"),
            "whole-name match"
        );
        assert!(
            !glob_match("refs/heads/ma*", "refs/heads/x"),
            "literal prefix"
        );
        assert!(glob_match("refs/heads/ma?n", "refs/heads/main"));
        assert!(
            !glob_match("refs/heads/ma?n", "refs/heads/maain"),
            "`?` is one char"
        );
        assert!(glob_match("*", "refs/anything/at/all"));
        assert!(!glob_match("refs/*", "refsx/a"));
    }

    #[cfg(unix)]
    #[test]
    fn oversized_file_is_rejected_before_parse() {
        let raw = vec![b'#'; FILE_MAX_BYTES + 1];
        let e = parse_and_validate(&raw).expect_err("must reject");
        assert!(e.contains("V9"), "{e}");
    }
}
