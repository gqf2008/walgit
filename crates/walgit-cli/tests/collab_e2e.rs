#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::dbg_macro
)]
//! D1 (`docs/D1_COLLAB_DESIGN.md` §4.3/§7): the `walgit collab` CLI against a
//! real walgit server — sign + push entries through receive-pack, then a
//! fresh clone aggregates and verifies them. Tests the whole loop: CLI write
//! path → walgit WAL → second-instance deterministic aggregation.

use std::path::Path;
use std::sync::Arc;

use walgit_config::{Config, StoreBackend};
use walgit_server::{AppState, router};
use walgit_store::DynStore;
use walgit_store::memory::MemoryStore;

type TestResult<T = ()> = anyhow::Result<T>;

fn git_in(dir: &Path, args: &[&str]) -> TestResult<String> {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()?;
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// In-process walgit server (mirrors the walgit-server test harness): memory
/// store, loopback `none` auth, `auto_create_on_push` so the CLI's first
/// `git push` creates the repository.
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
    cfg.wal.freshness_ttl = std::time::Duration::ZERO;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cli_full_collab_flow_against_a_real_server() -> TestResult {
    let (base, _shutdown) = start_server().await?;
    let bin = env!("CARGO_BIN_EXE_walgit");
    let keydir = tempfile::tempdir()?;
    let key = keydir.path().join("key");
    std::fs::write(&key, "07".repeat(32))?; // 32 raw bytes as hex

    let run = |args: &[&str]| -> TestResult<String> {
        let out = std::process::Command::new(bin)
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
    };
    let key_s = key.to_str().unwrap();

    // Scratch repo A: author/writer.
    let a = tempfile::tempdir()?;
    git_in(a.path(), &["init", "-q", "-b", "main"])?;
    git_in(a.path(), &["config", "user.email", "t@t"])?;
    git_in(a.path(), &["config", "user.name", "T"])?;
    git_in(
        a.path(),
        &["remote", "add", "origin", &format!("{base}/o/r.git")],
    )?;
    let repo_a = a.path().to_str().unwrap();

    // First-use registration, then a thread: issue -> comment (chained) ->
    // approve review; each pushed to the server through receive-pack.
    run(&[
        "collab", "principal-register", "--repo", repo_a, "--principal", "alice", "--key", key_s,
        "--push", "origin",
    ])?;
    let issue_out = run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "issue", "--id", "pr1", "--actor", "alice",
        "--body", r#"{"title":"add thing"}"#, "--key", key_s, "--push", "origin",
    ])?;
    let issue_oid = issue_out.split_whitespace().nth(1).unwrap().to_string();
    let comment_out = run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "comment", "--id", "pr1", "--actor", "alice",
        "--parent", &issue_oid, "--body", r#"{"note":"+1"}"#, "--key", key_s, "--push", "origin",
    ])?;
    let comment_oid = comment_out.split_whitespace().nth(1).unwrap().to_string();
    run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "review", "--id", "pr1", "--actor", "alice",
        "--parent", &comment_oid, "--body", r#"{"decision":"approve"}"#, "--key", key_s, "--push",
        "origin",
    ])?;

    // Second instance: a fresh clone fetches the collab refs and aggregates.
    let b = tempfile::tempdir()?;
    git_in(
        b.path(),
        &["clone", "-q", "--no-checkout", &format!("{base}/o/r.git"), "."],
    )?;
    git_in(b.path(), &["fetch", "-q", "origin", "+refs/collab/*:refs/collab/*"])?;
    let repo_b = b.path().to_str().unwrap();

    let thread_out = run(&["collab", "thread", "pr1", "--repo", repo_b])?;
    let thread: serde_json::Value = serde_json::from_str(&thread_out)?;
    let arr = thread.as_array().unwrap();
    assert_eq!(arr.len(), 3, "issue + comment + review: {thread_out}");
    assert!(
        arr.iter().all(|e| e["verified"] == serde_json::Value::Bool(true)),
        "all entries verify against the registry: {thread_out}"
    );
    assert_eq!(arr[0]["entry"]["kind"], "issue", "parent chain root first");
    assert_eq!(arr[1]["entry"]["kind"], "comment");
    assert_eq!(arr[2]["entry"]["kind"], "review");

    let pr_out = run(&["collab", "pr", "pr1", "--repo", repo_b])?;
    let pv: serde_json::Value = serde_json::from_str(&pr_out)?;
    assert_eq!(pv["pr"]["human_approvals"][0]["actor"], "alice");
    assert_eq!(pv["merge"]["allowed"], serde_json::Value::Bool(true));
    Ok(())
}
