#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::dbg_macro
)]
//! D1-CI (`docs/D1_CI_PROTOCOL.md`) end-to-end: real walgit server, real
//! `walgit ci run` processes as plain git clients. Covers the batch's
//! acceptance list — trigger → claim → execute → signed result (verified by
//! replay from a fresh instance), two runners competing on one event
//! converging to exactly one effective result, a killed claimer whose claim
//! expires and is re-claimed, the secret boundary (negative test against the
//! published entry objects), and the timeout conclusion.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use walgit_config::{Config, StoreBackend};
use walgit_server::{AppState, router};
use walgit_store::DynStore;
use walgit_store::memory::MemoryStore;

type TestResult<T = ()> = anyhow::Result<T>;

fn git_in(dir: &Path, args: &[&str]) -> TestResult<String> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// In-process walgit server (the `collab_e2e` harness): memory store, loopback
/// `none` auth, `auto_create_on_push` so the first `git push` creates the repo.
async fn start_server() -> TestResult<(String, tokio::sync::oneshot::Sender<()>)> {
    let store: Arc<MemoryStore> = MemoryStore::shared();
    let cache = tempfile::tempdir()?;
    let mut cfg = Config::default();
    cfg.store.backend = StoreBackend::Memory;
    cfg.store.bucket = "test".into();
    cfg.cache.dir = cache.path().to_path_buf();
    cfg.cache.max_bytes = bytesize::ByteSize::gib(2);
    cfg.server.listen = "127.0.0.1:0".parse().unwrap();
    cfg.server.auto_create_on_push = true;
    cfg.server.max_concurrent_per_repo = 8;
    cfg.server.max_push_bytes = bytesize::ByteSize::gib(2);
    cfg.wal.fsck_objects = true;
    cfg.wal.check_connectivity = true;
    cfg.wal.freshness_ttl = Duration::ZERO;
    let dyn_store: DynStore = store.clone();
    let state = AppState::new(Arc::new(cfg), dyn_store).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let app = router(state);
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = rx.await;
        })
        .await
        .ok();
    });
    Ok((format!("http://{addr}"), tx))
}

/// Deterministic signing key, one per runner (the same hex form
/// `walgit collab principal-register` takes).
fn write_key(dir: &Path, seed: u8) -> TestResult<String> {
    let key = dir.join(format!("key-{seed}"));
    std::fs::write(&key, format!("{seed:02x}").repeat(32))?;
    Ok(key.to_str().unwrap().to_string())
}

/// A working repo pushed to the server with `.walgit/ci.toml` at its tip.
fn work_repo(base: &str, ci_toml: &str) -> TestResult<tempfile::TempDir> {
    let d = tempfile::tempdir()?;
    git_in(d.path(), &["init", "-q", "-b", "main"])?;
    git_in(d.path(), &["config", "user.email", "t@t"])?;
    git_in(d.path(), &["config", "user.name", "T"])?;
    git_in(
        d.path(),
        &["remote", "add", "origin", &format!("{base}/o/ci.git")],
    )?;
    std::fs::create_dir_all(d.path().join(".walgit"))?;
    std::fs::write(d.path().join(".walgit/ci.toml"), ci_toml)?;
    git_in(d.path(), &["add", "."])?;
    git_in(
        d.path(),
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "declare ci",
        ],
    )?;
    git_in(d.path(), &["push", "-q", "origin", "main"])?;
    Ok(d)
}

/// A runner checkout: a full clone (the runner executes in worktrees of it).
fn clone_repo(base: &str) -> TestResult<tempfile::TempDir> {
    let d = tempfile::tempdir()?;
    git_in(d.path(), &["clone", "-q", &format!("{base}/o/ci.git"), "."])?;
    Ok(d)
}

fn path_s(d: &tempfile::TempDir) -> &str {
    d.path().to_str().unwrap()
}

/// `walgit ci run --once ...` — the raw output, so each test asserts its own
/// exit-code contract.
fn ci_run(
    bin: &str,
    repo: &str,
    actor: &str,
    key: &str,
    extra: &[&str],
    env: &[(&str, &str)],
) -> TestResult<std::process::Output> {
    let mut cmd = Command::new(bin);
    cmd.arg("--config")
        .arg("/dev/null")
        .args([
            "ci", "run", "--repo", repo, "--remote", "origin", "--actor", actor, "--key", key,
        ])
        .arg("--once")
        .args(extra);
    for (k, v) in env {
        cmd.env(k, v);
    }
    Ok(cmd.output()?)
}

fn run(bin: &str, args: &[&str]) -> TestResult<String> {
    let out = Command::new(bin)
        .arg("--config")
        .arg("/dev/null")
        .args(args)
        .output()?;
    assert!(
        out.status.success(),
        "walgit {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// `walgit ci status --format json` over the clone's local collab refs.
fn status_json(bin: &str, repo: &str) -> TestResult<serde_json::Value> {
    let out = run(bin, &["ci", "status", "--repo", repo, "--format", "json"])?;
    Ok(serde_json::from_str(&out)?)
}

/// Pull every collab ref fresh from the server into the clone.
fn refresh_collab(repo: &Path) -> TestResult<()> {
    git_in(
        repo,
        &["fetch", "-q", "origin", "+refs/collab/*:refs/collab/*"],
    )?;
    Ok(())
}

/// One run's view out of a status document, by task name.
fn run_of<'a>(st: &'a serde_json::Value, task: &str) -> &'a serde_json::Value {
    st["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["task"] == task)
        .unwrap_or_else(|| panic!("run {task} missing: {st}"))
}

/// How many attempts carry an effective result — the protocol's whole point
/// is that this is exactly 1 no matter how many runners executed (§7.2).
fn effective_count(run: &serde_json::Value) -> usize {
    run["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| !a["effective"].is_null())
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runner_executes_declared_tasks_and_results_verify_by_replay() -> TestResult {
    let (base, _shutdown) = start_server().await?;
    let bin = env!("CARGO_BIN_EXE_walgit").to_string();
    let keydir = tempfile::tempdir()?;
    let key = write_key(keydir.path(), 0x11)?;

    let ci_toml = r#"
version = 1
[[task]]
name = "build"
command = "echo hello-from-ci"
[[task]]
name = "tagbuild"
refs = ["refs/tags/v*"]
command = "echo tagged-ok"
"#;
    let work = work_repo(&base, ci_toml)?;
    // The declaration is validated against the working tree before the push
    // mattered — smoke the validate surface end-to-end as well.
    run(&bin, &["ci", "validate", "--repo", path_s(&work)])?;

    let r1 = clone_repo(&base)?;
    let r1s = path_s(&r1).to_string();

    // One changed ref, one run, one signed result.
    let out = ci_run(&bin, &r1s, "ci-a", &key, &[], &[])?;
    assert!(
        out.status.success(),
        "runner pass failed: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let st = status_json(&bin, &r1s)?;
    let runs = st["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "one changed ref, one run: {st}");
    assert_eq!(runs[0]["task"], "build");
    assert_eq!(runs[0]["state"], "done");
    assert_eq!(runs[0]["conclusion"], "success");
    assert_eq!(runs[0]["runner"], "ci-a");
    assert_eq!(effective_count(&runs[0]), 1);
    let run_id = runs[0]["id"].as_str().unwrap().to_string();
    assert!(run_id.starts_with("ci-") && run_id.len() == 19, "{run_id}");

    // A tag push is a second event; only the refs/tags/v* task matches it.
    git_in(work.path(), &["tag", "v1"])?;
    git_in(work.path(), &["push", "-q", "origin", "v1"])?;
    let out = ci_run(&bin, &r1s, "ci-a", &key, &[], &[])?;
    assert!(out.status.success(), "second pass failed");
    let st = status_json(&bin, &r1s)?;
    let runs = st["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "tag event triggered its own run: {st}");
    let tag_run = runs.iter().find(|r| r["task"] == "tagbuild").unwrap();
    assert_eq!(tag_run["conclusion"], "success");
    // The already-processed main tip was not re-run.
    assert_eq!(effective_count(run_of(&st, "build")), 1);

    // Replay from a fresh instance: the thread is claim → result, both
    // verifying against the registry, the result hanging off its claim.
    let fresh = clone_repo(&base)?;
    refresh_collab(fresh.path())?;
    let thread = run(
        &bin,
        &["collab", "thread", &run_id, "--repo", path_s(&fresh)],
    )?;
    let arr = serde_json::from_str::<serde_json::Value>(&thread)?;
    let arr = arr.as_array().unwrap();
    assert_eq!(arr.len(), 2, "claim + result: {thread}");
    assert!(
        arr.iter()
            .all(|e| e["verified"] == serde_json::Value::Bool(true)),
        "both entries verify: {thread}"
    );
    assert_eq!(arr[0]["entry"]["kind"], "ci_claim");
    assert_eq!(arr[0]["entry"]["body"]["task"], "build");
    assert_eq!(arr[0]["entry"]["body"]["ttl"], 300);
    assert_eq!(arr[1]["entry"]["kind"], "ci_result");
    assert_eq!(
        arr[1]["entry"]["parent"], arr[0]["oid"],
        "result hangs off its claim"
    );
    assert_eq!(arr[1]["entry"]["body"]["claim"], arr[0]["oid"]);
    assert_eq!(arr[1]["entry"]["body"]["conclusion"], "success");
    assert_eq!(arr[1]["entry"]["body"]["exit_code"], 0);
    assert!(
        arr[1]["entry"]["body"]["log_summary"]
            .as_str()
            .unwrap()
            .contains("hello-from-ci")
    );
    assert_eq!(
        arr[1]["entry"]["body"]["log_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );

    // A third pass is idempotent: settled runs are not re-run.
    let out = ci_run(&bin, &r1s, "ci-a", &key, &[], &[])?;
    assert!(out.status.success());
    let st = status_json(&bin, &r1s)?;
    assert_eq!(st["runs"].as_array().unwrap().len(), 2, "no re-runs: {st}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_runners_competing_converge_to_one_effective_result() -> TestResult {
    let (base, _shutdown) = start_server().await?;
    let bin = env!("CARGO_BIN_EXE_walgit").to_string();
    let keydir = tempfile::tempdir()?;
    let key_a = write_key(keydir.path(), 0x21)?;
    let key_b = write_key(keydir.path(), 0x22)?;

    let ci_toml = r#"
version = 1
[[task]]
name = "build"
command = "echo ran"
"#;
    let _work = work_repo(&base, ci_toml)?;
    let r1 = clone_repo(&base)?;
    let r2 = clone_repo(&base)?;
    let r1s = path_s(&r1).to_string();
    let r2s = path_s(&r2).to_string();

    // Two runners pick up the same event simultaneously (§6.2: they may both
    // claim; the deterministic winner rule decides who executes — and exactly
    // one result is effective regardless of how the race lands).
    let (out_a, out_b) = std::thread::scope(|s| {
        let ha = s.spawn(|| ci_run(&bin, &r1s, "ci-a", &key_a, &[], &[]));
        let hb = s.spawn(|| ci_run(&bin, &r2s, "ci-b", &key_b, &[], &[]));
        (ha.join().unwrap().unwrap(), hb.join().unwrap().unwrap())
    });
    assert!(
        out_a.status.success(),
        "runner a: {}",
        String::from_utf8_lossy(&out_a.stderr)
    );
    assert!(
        out_b.status.success(),
        "runner b: {}",
        String::from_utf8_lossy(&out_b.stderr)
    );

    // Any client, fresh from the server, computes the same answer (§1).
    let fresh = clone_repo(&base)?;
    refresh_collab(fresh.path())?;
    let st = status_json(&bin, path_s(&fresh))?;
    let build = run_of(&st, "build");
    assert_eq!(build["state"], "done", "{st}");
    assert_eq!(build["conclusion"], "success", "{st}");
    assert_eq!(
        effective_count(build),
        1,
        "exactly one effective result: {st}"
    );
    let attempts = build["attempts"].as_array().unwrap();
    let results = attempts[0]["results"].as_array().unwrap();
    assert!(!results.is_empty(), "at least the winner published: {st}");
    assert!(
        results.len() <= 2,
        "at most one result per competing runner: {st}"
    );
    // The effective result is the one answering the winning claim (§7.2).
    assert_eq!(
        attempts[0]["effective"]["claim"], attempts[0]["winner"]["oid"],
        "{st}"
    );
    // Both entries verify, whoever executed.
    let run_id = build["id"].as_str().unwrap();
    let thread = run(
        &bin,
        &["collab", "thread", run_id, "--repo", path_s(&fresh)],
    )?;
    let arr = serde_json::from_str::<serde_json::Value>(&thread)?;
    assert!(
        arr.as_array()
            .unwrap()
            .iter()
            .all(|e| e["verified"] == serde_json::Value::Bool(true)),
        "{thread}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn killed_runner_loses_claim_to_ttl_and_the_run_recovers() -> TestResult {
    let (base, _shutdown) = start_server().await?;
    let bin = env!("CARGO_BIN_EXE_walgit").to_string();
    let keydir = tempfile::tempdir()?;
    let key_a = write_key(keydir.path(), 0x31)?;
    let key_b = write_key(keydir.path(), 0x32)?;

    // The task runs long enough for the test to kill its runner mid-execution.
    let ci_toml = r#"
version = 1
[[task]]
name = "build"
command = "sleep 3"
timeout = "60s"
"#;
    let _work = work_repo(&base, ci_toml)?;
    let r1 = clone_repo(&base)?;
    let r2 = clone_repo(&base)?;
    let r1s = path_s(&r1).to_string();
    let r2s = path_s(&r2).to_string();

    // The victim claims and is killed mid-run (§6.3: no result, no cleanup,
    // the claim simply expires).
    let mut victim = Command::new(&bin)
        .arg("--config")
        .arg("/dev/null")
        .args([
            "ci", "run", "--repo", &r1s, "--remote", "origin", "--actor", "ci-a",
        ])
        .arg("--key")
        .arg(&key_a)
        .arg("--once")
        .args(["--claim-ttl", "5"])
        .spawn()?;
    let mut claimed = false;
    for _ in 0..100 {
        let out = git_in(
            r2.path(),
            &["ls-remote", "origin", "refs/collab/inbox/ci-a/*"],
        )?;
        if !out.trim().is_empty() {
            claimed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(claimed, "the victim published its claim");
    victim.kill()?;
    victim.wait()?;

    // The rescuer's first pass stands down: the claim is still live (§7.3).
    let out = ci_run(&bin, &r2s, "ci-b", &key_b, &["--claim-ttl", "5"], &[])?;
    assert!(out.status.success(), "stand-down pass is not a failure");
    let st = status_json(&bin, &r2s)?;
    let build = run_of(&st, "build");
    assert_eq!(build["state"], "claimed", "{st}");
    assert_eq!(build["runner"], "ci-a", "{st}");
    assert_eq!(
        effective_count(build),
        0,
        "no result from the dead runner: {st}"
    );

    // Wait out the TTL: the same local entries go stale as the clock passes
    // ts + ttl — no fetch needed, expiry is a pure function of `now` (§6.4).
    for _ in 0..80 {
        let st = status_json(&bin, &r2s)?;
        if run_of(&st, "build")["state"] == "stale" {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let st = status_json(&bin, &r2s)?;
    assert_eq!(run_of(&st, "build")["state"], "stale", "{st}");

    // Re-claim, execute, publish (§6.3): the run completes on the same attempt.
    let out = ci_run(&bin, &r2s, "ci-b", &key_b, &["--claim-ttl", "5"], &[])?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let st = status_json(&bin, &r2s)?;
    let build = run_of(&st, "build");
    assert_eq!(build["state"], "done", "{st}");
    assert_eq!(build["conclusion"], "success", "{st}");
    let attempts = build["attempts"].as_array().unwrap();
    assert_eq!(
        attempts.len(),
        1,
        "re-claim of the same attempt, not a retry: {st}"
    );
    assert_eq!(
        attempts[0]["claims"].as_array().unwrap().len(),
        2,
        "old claim + new claim: {st}"
    );
    assert_eq!(attempts[0]["effective"]["actor"], "ci-b", "{st}");
    assert_eq!(effective_count(build), 1, "{st}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_secret_boundary_holds_in_every_published_object() -> TestResult {
    const SECRET: &str = "s3cret-e2e-value-do-not-publish";
    let (base, _shutdown) = start_server().await?;
    let bin = env!("CARGO_BIN_EXE_walgit").to_string();
    let keydir = tempfile::tempdir()?;
    let key = write_key(keydir.path(), 0x41)?;

    // The allowlist carries only NAMES. The command itself is the positive
    // guard: the allowed variable is truly injected, the secret truly absent —
    // if the boundary leaked, this run would settle failure, not success.
    let ci_toml = r#"
version = 1
[[task]]
name = "build"
command = "test \"$CI_E2E_ALLOWED\" = yes && test -z \"$CI_E2E_SECRET\" && echo clean"
env_allow = ["CI_E2E_ALLOWED"]
"#;
    let _work = work_repo(&base, ci_toml)?;
    let r1 = clone_repo(&base)?;
    let r1s = path_s(&r1).to_string();

    // The runner's own environment holds the secret — as a real runner's would.
    let out = ci_run(
        &bin,
        &r1s,
        "ci-a",
        &key,
        &[],
        &[("CI_E2E_SECRET", SECRET), ("CI_E2E_ALLOWED", "yes")],
    )?;
    assert!(
        out.status.success(),
        "the guard failed — the allowlist does not work: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // Every published surface is scanned for the secret value (§9.4): the
    // claim blob, the result blob, the thread JSON, the status JSON, and the
    // runner's own output.
    refresh_collab(r1.path())?;
    let st = status_json(&bin, &r1s)?;
    let run_id = run_of(&st, "build")["id"].as_str().unwrap().to_string();
    let thread = run(&bin, &["collab", "thread", &run_id, "--repo", &r1s])?;
    let arr = serde_json::from_str::<serde_json::Value>(&thread)?;
    assert_eq!(arr.as_array().unwrap().len(), 2, "claim + result: {thread}");

    // The raw entry blobs: claim and result straight out of the object base.
    let mut blobs = String::new();
    for e in arr.as_array().unwrap() {
        let oid = e["oid"].as_str().unwrap();
        blobs.push_str(&git_in(r1.path(), &["cat-file", "blob", oid])?);
    }
    let status_text = st.to_string();
    let stdout_text = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr_text = String::from_utf8_lossy(&out.stderr).into_owned();
    let surfaces: [(&str, &str); 5] = [
        ("thread", thread.as_str()),
        ("status", status_text.as_str()),
        ("blobs", blobs.as_str()),
        ("runner stdout", stdout_text.as_str()),
        ("runner stderr", stderr_text.as_str()),
    ];
    for (name, text) in surfaces {
        assert!(
            !text.contains(SECRET),
            "{name} leaks the secret value (§9 boundary broken)"
        );
    }
    assert_eq!(run_of(&st, "build")["conclusion"], "success", "{st}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_that_outlives_its_timeout_settles_timeout() -> TestResult {
    let (base, _shutdown) = start_server().await?;
    let bin = env!("CARGO_BIN_EXE_walgit").to_string();
    let keydir = tempfile::tempdir()?;
    let key = write_key(keydir.path(), 0x51)?;

    let ci_toml = r#"
version = 1
[[task]]
name = "build"
command = "sleep 30"
timeout = "1s"
"#;
    let _work = work_repo(&base, ci_toml)?;
    let r1 = clone_repo(&base)?;
    let r1s = path_s(&r1).to_string();

    // A timeout is a verdict, not an infra error: the result publishes, the
    // pass exits non-zero, exit_code is null (§8.2).
    let out = ci_run(&bin, &r1s, "ci-a", &key, &[], &[])?;
    assert!(!out.status.success(), "a timed-out task fails the pass");
    let st = status_json(&bin, &r1s)?;
    let build = run_of(&st, "build");
    assert_eq!(build["state"], "done", "{st}");
    assert_eq!(build["conclusion"], "timeout", "{st}");
    let attempts = build["attempts"].as_array().unwrap();
    assert_eq!(
        attempts[0]["effective"]["exit_code"],
        serde_json::Value::Null,
        "{st}"
    );
    // The captured log is empty (sleep prints nothing) — sha of empty, no lie.
    assert_eq!(attempts[0]["effective"]["log_summary"], "", "{st}");
    Ok(())
}
