//! The first-run setup wizard (D43): setup-state gating, the open wizard
//! API, and save write-back into the running config file.

mod harness;

use harness::*;
use std::sync::Arc;

type TestResult = anyhow::Result<()>;

/// Setup state (memory without the deliberate-use flag): only the wizard
/// surface answers — the status API, `/` → `/setup`, liveness probes — and
/// everything else is a 503 that names `/setup`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn setup_state_serves_only_the_wizard() -> TestResult {
    let server = Server::start_with_tweak(|c| {
        c.store.memory_backend_intentional = false;
    })
    .await?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    // The wizard API is open and truthful.
    let r = client
        .get(format!("{}/api/v1/setup/status", server.base_url))
        .send()
        .await?;
    assert_eq!(r.status(), 200, "{}", r.status());
    let v: serde_json::Value = r.json().await?;
    assert_eq!(v["needs_setup"], true, "{v}");
    assert_eq!(v["backend"], "memory", "{v}");
    assert_eq!(
        v["can_save"], false,
        "harness servers run without a config file"
    );

    // `/` redirects to the wizard page.
    let r = client.get(&server.base_url).send().await?;
    assert!(r.status().is_redirection(), "{}", r.status());
    assert!(
        r.headers()
            .get("location")
            .unwrap()
            .to_str()?
            .ends_with("/setup")
    );

    // The wizard page shell answers (assets present after `pnpm run build`).
    let r = client
        .get(format!("{}/setup", server.base_url))
        .send()
        .await?;
    assert_eq!(r.status(), 200, "{}", r.status());

    // Liveness probes stay open.
    let r = client
        .get(format!("{}/healthz", server.base_url))
        .send()
        .await?;
    assert_eq!(r.status(), 200, "{}", r.status());

    // Everything else: 503 with a pointer.
    for path in [
        "/t/x.git/info/refs?service=git-upload-pack",
        "/api/v1/owners",
        "/t/x/api/refs",
    ] {
        let r = client
            .get(format!("{}{}", server.base_url, path))
            .send()
            .await?;
        assert_eq!(r.status(), 503, "{path}: {}", r.status());
        assert!(r.text().await?.contains("/setup"), "{path}");
    }
    Ok(())
}

/// Configured (the harness default): the setup API answers 404 and normal
/// serving is untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_instance_answers_404_on_the_setup_api() -> TestResult {
    let server = Server::start().await?;
    let client = reqwest::Client::new();
    let r = client
        .get(format!("{}/api/v1/setup/status", server.base_url))
        .send()
        .await?;
    assert_eq!(r.status(), 404, "{}", r.status());
    // Normal serving works: the repo API answers (no repo → 404 on the repo,
    // not on the gateway).
    let r = client
        .get(format!("{}/api/v1/owners", server.base_url))
        .send()
        .await?;
    assert_eq!(r.status(), 200, "{}", r.status());
    Ok(())
}

/// Save writes the composed config back into the running file — comments and
/// unrelated keys survive, the result parses and validates, and the response
/// reports the manual restart (harness servers never exit).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn save_writes_back_and_preserves_comments() -> TestResult {
    let dir = tempfile::tempdir()?;
    let cfg_path = dir.path().join("walgit.toml");
    std::fs::write(
        &cfg_path,
        "# 顶注释保留\n[server]\nlisten = \"127.0.0.1:8081\"\n\n[server.auth]\nmode = \"none\"\n\n[store]\n# 内存后端\nbackend = \"memory\"\n",
    )?;
    let server = Server::start_setup(&cfg_path).await?;
    let client = reqwest::Client::new();

    let r = client
        .post(format!("{}/api/v1/setup/save", server.base_url))
        .json(&serde_json::json!({
            "store": {
                "backend": "s3",
                "bucket": "my-bucket",
                "endpoint": "https://acct.r2.cloudflarestorage.com",
                "region": "auto",
                "access_key": "AK",
                "secret_key": "SK",
                "force_path_style": true
            },
            "admin_token": "wgt-first-admin-token"
        }))
        .send()
        .await?;
    assert_eq!(r.status(), 200, "{}", r.text().await?);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["saved"], true, "{body}");
    assert_eq!(
        body["restart"], "manual",
        "harness servers never exit — {body}"
    );

    let text = std::fs::read_to_string(&cfg_path)?;
    assert!(text.contains("# 顶注释保留"), "{text}");
    assert!(text.contains("# 内存后端"), "{text}");
    assert!(text.contains("backend = \"s3\""), "{text}");
    assert!(text.contains("bucket = \"my-bucket\""), "{text}");
    assert!(text.contains("access_key = \"AK\""), "{text}");
    assert!(text.contains("mode = \"token\""), "{text}");
    assert!(text.contains("principal = \"admin\""), "{text}");
    let cfg: walgit_config::Config = toml::from_str(&text).unwrap();
    assert!(matches!(cfg.store.backend, walgit_config::StoreBackend::S3));
    assert_eq!(cfg.store.s3.access_key.as_deref(), Some("AK"));
    assert_eq!(cfg.server.auth.tokens.len(), 1);
    assert!(cfg.server.auth.tokens[0].admin);
    Ok(())
}

/// Save refuses an invalid composed config without touching the file.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn save_refuses_an_invalid_composed_config() -> TestResult {
    let dir = tempfile::tempdir()?;
    let cfg_path = dir.path().join("walgit.toml");
    std::fs::write(&cfg_path, "[store]\nbackend = \"memory\"\n")?;
    let before = std::fs::read_to_string(&cfg_path)?;
    let server = Server::start_setup(&cfg_path).await?;
    let client = reqwest::Client::new();

    let r = client
        .post(format!("{}/api/v1/setup/save", server.base_url))
        .json(&serde_json::json!({
            "store": { "backend": "s3", "bucket": "" } // empty bucket: validate fails
        }))
        .send()
        .await?;
    assert_eq!(r.status(), 400, "{}", r.status());
    assert_eq!(
        std::fs::read_to_string(&cfg_path)?,
        before,
        "a refused save must not touch the file"
    );
    Ok(())
}

/// Setup state refuses a non-loopback bind (fail-closed, §1.3's convention).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn setup_state_refuses_a_non_loopback_bind() -> TestResult {
    let store = walgit_store::memory::MemoryStore::shared();
    let mut cfg = walgit_config::Config::default();
    cfg.store.backend = walgit_config::StoreBackend::Memory;
    cfg.store.memory_backend_intentional = false;
    cfg.store.bucket = "test".into();
    cfg.server.listen = "0.0.0.0:0".parse().unwrap();
    let state = walgit_server::AppState::new(Arc::new(cfg), store).await?;
    let err = walgit_server::serve(state, std::future::pending::<()>())
        .await
        .unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("loopback"), "{text}");
    Ok(())
}
