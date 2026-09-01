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

    // Read-only observability dashboard over the same refs.
    let text = run(&["collab", "report", "--repo", repo_b])?;
    assert!(text.contains("collab report"), "{text}");
    assert!(text.contains("pr1"), "{text}");
    assert!(text.contains("verified"), "{text}");
    let md = run(&["collab", "report", "--repo", repo_b, "--format", "markdown"])?;
    assert!(md.contains("## PRs"), "{md}");
    let html = run(&["collab", "report", "--repo", repo_b, "--format", "html"])?;
    assert!(html.contains("<!doctype html>"), "{html}");
    assert!(html.contains("</html>"), "{html}");
    assert!(html.contains("pr1"), "{html}");
    Ok(())
}

/// `walgit collab watch`: cold start reports every collab ref, a later pass
/// reports only what is new, and the callback gets the entry JSON on stdin
/// with the env contract (kind/thread/actor/verified). Runs against a real
/// walgit server; the watch fetches refs/collab/* from a fresh clone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watch_reports_new_collab_entries_via_callback() -> TestResult {
    let (base, _shutdown) = start_server().await?;
    let bin = env!("CARGO_BIN_EXE_walgit");
    let keydir = tempfile::tempdir()?;
    let key = keydir.path().join("key");
    std::fs::write(&key, "07".repeat(32))?;
    let key_s = key.to_str().unwrap();

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

    // Repo A writes: register + issue entry, pushed to the server.
    let a = tempfile::tempdir()?;
    git_in(a.path(), &["init", "-q", "-b", "main"])?;
    git_in(a.path(), &["config", "user.email", "t@t"])?;
    git_in(a.path(), &["config", "user.name", "T"])?;
    git_in(
        a.path(),
        &["remote", "add", "origin", &format!("{base}/o/r.git")],
    )?;
    let repo_a = a.path().to_str().unwrap();
    run(&[
        "collab", "principal-register", "--repo", repo_a, "--principal", "alice", "--key", key_s,
        "--push", "origin",
    ])?;
    run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "issue", "--id", "wt1", "--actor", "alice",
        "--body", r#"{"title":"watch me"}"#, "--key", key_s, "--push", "origin",
    ])?;

    // Repo B: a fresh clone where the watcher lives.
    let b = tempfile::tempdir()?;
    git_in(
        b.path(),
        &["clone", "-q", "--no-checkout", &format!("{base}/o/r.git"), "."],
    )?;
    let repo_b = b.path().to_str().unwrap();
    let captured = tempfile::tempdir()?;
    let cap = captured.path().to_str().unwrap();
    let cb = format!("cat > {cap}/$WALGIT_COLLAB_KIND-$WALGIT_COLLAB_VERIFIED.txt");

    // Cold start: issue + principal are both reported through the callback.
    run(&[
        "collab", "watch", "--repo", repo_b, "--remote", "origin", "--once", "--exec", &cb,
    ])?;
    let issue_file = captured.path().join("issue-true.txt");
    let principal_file = captured.path().join("principal-true.txt");
    assert!(issue_file.exists(), "cold start delivered the issue entry");
    assert!(principal_file.exists(), "cold start delivered the principal record");
    let issue_v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&issue_file)?)?;
    assert_eq!(issue_v["kind"], "issue");
    assert_eq!(issue_v["id"], "wt1");

    // A second pass with no changes reports nothing new.
    run(&[
        "collab", "watch", "--repo", repo_b, "--remote", "origin", "--once", "--exec", &cb,
    ])?;
    let before = std::fs::read_to_string(&issue_file)?;
    std::thread::sleep(std::time::Duration::from_millis(1100)); // ensure distinct mtime
    run(&[
        "collab", "watch", "--repo", repo_b, "--remote", "origin", "--once", "--exec", &cb,
    ])?;
    assert_eq!(
        std::fs::read_to_string(&issue_file)?,
        before,
        "no new refs -> callback not re-fired"
    );

    // A new comment from another actor triggers only the comment callback.
    let comment_out = run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "comment", "--id", "wt1", "--actor",
        "alice", "--parent", "", "--body", r#"{"note":"from agent B"}"#, "--key", key_s,
        "--push", "origin",
    ])?;
    let comment_oid = comment_out.split_whitespace().nth(1).unwrap().to_string();
    // Chain it properly by rewriting the ref to point at the issue? Simpler: the
    // watcher only cares about new refs, so a fresh uuid ref is fine.
    let _ = comment_oid;
    run(&[
        "collab", "watch", "--repo", repo_b, "--remote", "origin", "--once", "--exec", &cb,
    ])?;
    let comment_file = captured.path().join("comment-true.txt");
    assert!(comment_file.exists(), "new comment delivered: {}", comment_file.display());
    let comment_v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&comment_file)?)?;
    assert_eq!(comment_v["kind"], "comment");
    Ok(())
}

/// The work-unit board (D1 §8): two independent clients — the CLI's offline
/// aggregation over a fetched clone and the server's `GET …/collab/board` —
/// must project the same collab refs to **byte-identical** output, and moving
/// a card (an ordinary signed `status` entry) must move the projection for
/// every client, verified against the registry.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn board_projection_is_byte_identical_across_clients_and_moves_with_status() -> TestResult {
    let (base, _shutdown) = start_server().await?;
    let bin = env!("CARGO_BIN_EXE_walgit");
    let keydir = tempfile::tempdir()?;
    let key = keydir.path().join("key");
    std::fs::write(&key, "07".repeat(32))?;
    let key_s = key.to_str().unwrap();

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

    // The board definition is versioned with the repository: the author
    // commits `.walgit/board.toml` to main before any collab traffic, so both
    // clients read the same committed definition.
    let board_toml = "version = 1\n\n[[column]]\nname = \"review\"\nstatus = \"needs-review\"\n\n[[column]]\nname = \"done\"\nstatus = \"merged\"\n\n[[column]]\nname = \"open\"\nstatus = \"open\"\n";
    let a = tempfile::tempdir()?;
    git_in(a.path(), &["init", "-q", "-b", "main"])?;
    git_in(a.path(), &["config", "user.email", "t@t"])?;
    git_in(a.path(), &["config", "user.name", "T"])?;
    git_in(
        a.path(),
        &["remote", "add", "origin", &format!("{base}/o/r.git")],
    )?;
    std::fs::write(a.path().join(".walgit-board.toml"), board_toml)?;
    std::fs::create_dir_all(a.path().join(".walgit"))?;
    std::fs::rename(a.path().join(".walgit-board.toml"), a.path().join(".walgit/board.toml"))?;
    git_in(a.path(), &["add", ".walgit/board.toml"])?;
    git_in(a.path(), &["commit", "-q", "-m", "board"])?;
    git_in(a.path(), &["push", "-q", "origin", "main"])?;
    let repo_a = a.path().to_str().unwrap();

    // Thread t1 walks onto the review lane; t2 stays open.
    run(&[
        "collab", "principal-register", "--repo", repo_a, "--principal", "alice", "--key", key_s,
        "--push", "origin",
    ])?;
    let issue_out = run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "issue", "--id", "t1", "--actor", "alice",
        "--body", r#"{"title":"add the thing"}"#, "--key", key_s, "--push", "origin",
    ])?;
    let issue_oid = issue_out.split_whitespace().nth(1).unwrap().to_string();
    let status_out = run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "status", "--id", "t1", "--actor", "alice",
        "--parent", &issue_oid, "--body", r#"{"status":"needs-review"}"#, "--key", key_s, "--push",
        "origin",
    ])?;
    let status_oid = status_out.split_whitespace().nth(1).unwrap().to_string();
    run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "review", "--id", "t1", "--actor", "alice",
        "--parent", &status_oid, "--body", r#"{"decision":"approve"}"#, "--key", key_s, "--push",
        "origin",
    ])?;
    run(&[
        "collab", "entry", "--repo", repo_a, "--kind", "issue", "--id", "t2", "--actor", "alice",
        "--body", r#"{"title":"open work"}"#, "--key", key_s, "--push", "origin",
    ])?;

    // Client 1: a fresh clone, aggregating offline from the fetched refs.
    let b = tempfile::tempdir()?;
    git_in(
        b.path(),
        &["clone", "-q", "--no-checkout", &format!("{base}/o/r.git"), "."],
    )?;
    git_in(b.path(), &["fetch", "-q", "origin", "+refs/collab/*:refs/collab/*"])?;
    let repo_b = b.path().to_str().unwrap();
    let cli_bytes = run(&["collab", "board", "--repo", repo_b, "--format", "json"])?;

    // Client 2: the server endpoint over HTTP.
    let resp = reqwest::get(format!("{base}/o/r/api/collab/board")).await?;
    assert_eq!(resp.status(), 200, "board endpoint status");
    let server_bytes = resp.bytes().await?;
    assert_eq!(
        cli_bytes.as_bytes(),
        &server_bytes[..],
        "CLI and server must project the same refs to identical bytes"
    );

    let board: serde_json::Value = serde_json::from_str(&cli_bytes)?;
    let column_of = |name: &str| -> Vec<String> {
        board["columns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .map(|c| {
                c["cards"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|card| card["id"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(column_of("review"), vec!["t1"], "status entry put t1 on review: {board}");
    assert_eq!(column_of("open"), vec!["t2"]);
    assert_eq!(column_of("done"), Vec::<String>::new());
    // text/markdown render the same projection (no need for byte equality).
    let text = run(&["collab", "board", "--repo", repo_b])?;
    assert!(text.contains("== review (1) =="), "{text}");
    let md = run(&["collab", "board", "--repo", repo_b, "--format", "markdown"])?;
    assert!(md.contains("## review (1)"), "{md}");

    // Move the t2 card: an ordinary signed `status` entry pushed to the
    // server's inbox — no second write semantics anywhere.
    let t2_out = run(&["collab", "thread", "t2", "--repo", repo_b])?;
    let t2: serde_json::Value = serde_json::from_str(&t2_out)?;
    let t2_tip = t2[0]["oid"].as_str().unwrap().to_string();
    run(&[
        "collab", "entry", "--repo", repo_b, "--kind", "status", "--id", "t2", "--actor", "alice",
        "--parent", &t2_tip, "--body", r#"{"status":"needs-review"}"#, "--key", key_s, "--push",
        "origin",
    ])?;

    // A third, independent clone sees the move: the new entry chains onto the
    // thread, verifies against the registry, and the projection moved.
    let c = tempfile::tempdir()?;
    git_in(
        c.path(),
        &["clone", "-q", "--no-checkout", &format!("{base}/o/r.git"), "."],
    )?;
    git_in(c.path(), &["fetch", "-q", "origin", "+refs/collab/*:refs/collab/*"])?;
    let repo_c = c.path().to_str().unwrap();
    let moved_thread = run(&["collab", "thread", "t2", "--repo", repo_c])?;
    let moved: serde_json::Value = serde_json::from_str(&moved_thread)?;
    let arr = moved.as_array().unwrap();
    assert_eq!(arr.len(), 2, "issue + the moving status entry: {moved_thread}");
    assert!(
        arr.iter().all(|e| e["verified"] == serde_json::Value::Bool(true)),
        "every entry verifies, including the move: {moved_thread}"
    );
    assert_eq!(arr[1]["entry"]["body"]["status"], "needs-review");

    let moved_cli = run(&["collab", "board", "--repo", repo_c, "--format", "json"])?;
    let moved_server = reqwest::get(format!("{base}/o/r/api/collab/board")).await?.bytes().await?;
    assert_eq!(moved_cli.as_bytes(), &moved_server[..], "post-move: still byte-identical");
    let moved_board: serde_json::Value = serde_json::from_str(&moved_cli)?;
    let review = moved_board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "review")
        .unwrap()["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    // Default sort is last-activity descending: the just-moved t2 leads.
    let mut review_sorted = review.clone();
    review_sorted.sort();
    assert_eq!(review_sorted, vec!["t1", "t2"], "the move put t2 on review: {moved_board}");
    assert_eq!(review.first(), Some(&"t2".to_string()), "newest activity first: {moved_board}");

    // Fail closed: an unparseable definition must be a loud error everywhere —
    // the CLI refuses to render, the server answers 400 — never a silent
    // fallback to the default board (that would hide a typo'd column rule).
    let bad = "version = 1\n\n[[column]]\nname = \"x\"\nbogus = true\n";
    let bad_dir = tempfile::tempdir()?;
    let bad_file = bad_dir.path().join("bad.toml");
    std::fs::write(&bad_file, bad)?;
    let bad_out = std::process::Command::new(bin)
        .arg("--config")
        .arg("/dev/null")
        .args([
            "collab",
            "board",
            "--repo",
            repo_b,
            "--format",
            "json",
            "--board",
            bad_file.to_str().unwrap(),
        ])
        .output()?;
    assert!(
        !bad_out.status.success(),
        "CLI must refuse a definition with an unknown field: {}",
        String::from_utf8_lossy(&bad_out.stdout)
    );
    std::fs::write(a.path().join(".walgit/board.toml"), bad)?;
    git_in(a.path(), &["commit", "-qam", "break the board"])?;
    git_in(a.path(), &["push", "-q", "origin", "main"])?;
    let bad_resp = reqwest::get(format!("{base}/o/r/api/collab/board")).await?;
    assert_eq!(
        bad_resp.status(),
        400,
        "server must refuse a repo whose HEAD definition does not parse"
    );
    Ok(())
}
