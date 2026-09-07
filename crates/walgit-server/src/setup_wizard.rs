//! The first-run setup wizard (D43, issue #70).
//!
//! A `memory` store without `memory_backend_intentional` means "unconfigured"
//! (the installer's initial config ships exactly that shape): the instance
//! serves only the wizard — `/setup` (a client route of the SPA shell) and its
//! `/_ui/*` assets, the data-free `/api/v1/setup/*` API (same open rule as
//! `/_auth/*`), and the liveness probes — while every other path answers 503
//! with a pointer to `/setup`. Setup state additionally requires loopback
//! listening (refused at startup otherwise, fail-closed like §1.3).
//!
//! The wizard configures object storage (S3/R2 — R2 is S3-compatible — or
//! GCS with ADC), tests the bucket, and writes everything — store section and
//! the first admin token — back into `walgit.toml` via `toml_edit`, which
//! preserves the file's comments and unrelated structure. The CLI serve path
//! then exits 75 for the supervisor (the tray watcher, D43) to restart into
//! the configured instance.
//!
//! Once configured (`backend != memory` or the flag set), `needs_setup` is
//! false, the gate is not mounted and `/api/v1/setup/*` answers 404.

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::web::ui;
use crate::{AppState, error::ApiError};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/setup/status", get(status))
        .route("/api/v1/setup/test", post(test_connection))
        .route("/api/v1/setup/save", post(save))
        .with_state(state)
}

/// The outermost gate while the instance is in setup state (mounted only when
/// `needs_setup`, so the configured path never pays for it). Everything the
/// wizard needs is answered here, ahead of the auth and compression layers;
/// the setup API passes through to its own router; all else is a 503 that
/// names `/setup`.
pub async fn gate(req: Request<Body>, next: axum::middleware::Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    // Liveness probes stay open (a supervisor must tell the wizard instance is alive).
    if path == "/healthz" || path == "/readyz" {
        return next.run(req).await;
    }
    if path == "/api/v1/setup" || path.starts_with("/api/v1/setup/") {
        return next.run(req).await;
    }
    // The wizard page is a client route of the SPA shell; its assets ride the
    // normal immutable/no-cache asset rules. Served here — the auth layer
    // would 401: setup state has no credentials yet.
    if path == "/setup" {
        return ui::shell(&method, req.headers());
    }
    if let Some(asset_path) = path.strip_prefix("/_ui/") {
        return ui::asset_bytes(asset_path, &method, req.headers());
    }
    if path == "/" {
        return Redirect::temporary("/setup").into_response();
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "walgit: setup required — object storage is not configured yet; open /setup to run the first-run wizard",
    )
        .into_response()
}

/// `GET /api/v1/setup/status` — why the wizard owns this instance and what it
/// can do. 404 once configured.
#[derive(Serialize)]
struct Status {
    needs_setup: bool,
    backend: &'static str,
    auth_mode: &'static str,
    can_save: bool,
}

async fn status(State(st): State<Arc<AppState>>) -> Response {
    if !st.needs_setup {
        return StatusCode::NOT_FOUND.into_response();
    }
    let auth_mode = match st.cfg.server.auth.mode {
        walgit_config::AuthMode::None => "none",
        walgit_config::AuthMode::Token => "token",
        walgit_config::AuthMode::Oidc => "oidc",
    };
    axum::Json(Status {
        needs_setup: true,
        backend: "memory",
        auth_mode,
        can_save: st.config_path.is_some(),
    })
    .into_response()
}

/// What the wizard offers the user to configure (steps ① + ③ of the issue).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupStore {
    /// `s3` (R2 is S3-compatible) or `gcs`.
    backend: String,
    bucket: String,
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    region: String,
    /// Literal credentials (D43). Empty = the `*_env` names in the config win
    /// (kept as-is) — the S3-only fields are ignored for `gcs`.
    #[serde(default)]
    access_key: String,
    #[serde(default)]
    secret_key: String,
    #[serde(default = "default_true")]
    force_path_style: bool,
}

fn default_true() -> bool {
    true
}

/// Apply the submitted store to a scratch config — shared by the test
/// connection (never persisted) and the save (persisted after validation).
fn apply_store(cfg: &mut walgit_config::Config, store: &SetupStore) -> Result<(), ApiError> {
    if store.bucket.trim().is_empty() {
        return Err(ApiError::BadRequest("bucket is required".into()));
    }
    cfg.store.bucket = store.bucket.trim().to_string();
    match store.backend.as_str() {
        "s3" => {
            cfg.store.backend = walgit_config::StoreBackend::S3;
            let s3 = &mut cfg.store.s3;
            s3.endpoint = store.endpoint.trim().to_string();
            s3.region = if store.region.trim().is_empty() {
                "auto".to_string()
            } else {
                store.region.trim().to_string()
            };
            s3.access_key =
                (!store.access_key.trim().is_empty()).then(|| store.access_key.trim().to_string());
            s3.secret_key =
                (!store.secret_key.trim().is_empty()).then(|| store.secret_key.trim().to_string());
            s3.force_path_style = store.force_path_style;
        }
        "gcs" => {
            cfg.store.backend = walgit_config::StoreBackend::Gcs;
            // Credentials ride ADC/IAM; only the gRPC endpoint (an emulator,
            // or empty = real GCS) and the bucket are wizard concerns.
            cfg.store.gcs.endpoint = store.endpoint.trim().to_string();
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "store backend must be \"s3\" or \"gcs\", got {other:?}"
            )));
        }
    }
    Ok(())
}

/// `POST /api/v1/setup/test` — build a store from the submitted params (never
/// persisted) and probe the bucket: HEAD of a key that cannot exist yet. A
/// `NotFound` *is* success (credentials + bucket reachable, no such object); a
/// success is success too; anything else is the surfaced error.
async fn test_connection(State(st): State<Arc<AppState>>, body: Body) -> Response {
    if !st.needs_setup {
        return StatusCode::NOT_FOUND.into_response();
    }
    let bytes = match crate::collect_body(body).await {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let store: SetupStore = match serde_json::from_slice(&bytes) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid setup payload: {e}"),
            )
                .into_response();
        }
    };
    let mut scratch = walgit_config::Config::default();
    if let Err(e) = apply_store(&mut scratch, &store) {
        return e.into_response();
    }
    let backend = match scratch.store.backend {
        walgit_config::StoreBackend::Gcs => "gcs",
        walgit_config::StoreBackend::S3 => "s3",
        walgit_config::StoreBackend::Memory => "memory",
    };
    match walgit_store::open_store(&scratch).await {
        Ok(opened) => {
            let probe_key = format!("{}setup-probe", scratch.store_prefix());
            match opened.head(&probe_key).await {
                Ok(_) | Err(walgit_store::StoreError::NotFound { .. }) => (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "ok": true,
                        "backend": backend,
                        "bucket": scratch.store.bucket,
                        "message": "bucket reachable with these credentials",
                    })),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "ok": false,
                        "backend": backend,
                        "message": format!("{e:#}"),
                    })),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "ok": false,
                "backend": backend,
                "message": format!("{e:#}"),
            })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupSave {
    store: SetupStore,
    /// The first admin (step ③). Written only when the current auth mode is
    /// `none` — an already-locked-down config is left to its own admin tooling.
    #[serde(default)]
    admin_token: Option<String>,
    #[serde(default)]
    admin_principal: Option<String>,
}

/// `POST /api/v1/setup/save` — validate the composed config, then edit the
/// running `walgit.toml` in place with `toml_edit` (comments and unrelated
/// keys survive), and — when the CLI serve path armed it — exit 75 so the
/// supervisor restarts into the configured instance.
async fn save(State(st): State<Arc<AppState>>, body: Body) -> Response {
    if !st.needs_setup {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(config_path) = st.config_path.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "this instance was not started from a config file (start with walgit serve --config walgit.toml); saved nothing",
        )
            .into_response();
    };
    let bytes = match crate::collect_body(body).await {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let req: SetupSave = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid setup payload: {e}"),
            )
                .into_response();
        }
    };
    let mut warnings = Vec::new();

    // 1. Compose + validate a scratch config *before* touching the file.
    let mut scratch = walgit_config::Config::default();
    if let Err(e) = apply_store(&mut scratch, &req.store) {
        return e.into_response();
    }
    // The admin step applies only when the running auth is `none` (the wizard
    // bootstraps the *first* identity). An already-configured auth is left to
    // its own admin tooling — the same decision must govern the file edit,
    // not just this scratch validation (a divergent edit would write a live
    // admin token while telling the user it was skipped).
    let write_admin = req
        .admin_token
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty())
        && st.cfg.server.auth.mode == walgit_config::AuthMode::None;
    if req
        .admin_token
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty())
        && !write_admin
    {
        warnings.push(
            "auth is already configured (not `none`); the admin step was skipped".to_string(),
        );
    }
    if write_admin {
        scratch.server.auth.mode = walgit_config::AuthMode::Token;
        scratch.server.auth.tokens = vec![walgit_config::StaticToken {
            principal: req
                .admin_principal
                .as_deref()
                .filter(|p| !p.trim().is_empty())
                .unwrap_or("admin")
                .to_string(),
            token: req
                .admin_token
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            token_env: None,
            write: true,
            admin: true,
        }];
    }
    if let Err(e) = scratch.validate() {
        return (
            StatusCode::BAD_REQUEST,
            format!("the composed config does not validate: {e:#}"),
        )
            .into_response();
    }

    // 2. Edit the running file in place, preserving everything else.
    let text = match std::fs::read_to_string(&config_path) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("config file {} is unreadable: {e}", config_path.display()),
            )
                .into_response();
        }
    };
    let mut doc: toml_edit::DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!(
                    "config file {} is not valid TOML: {e}",
                    config_path.display()
                ),
            )
                .into_response();
        }
    };
    if let Err(e) = edit_document(&mut doc, &req, write_admin) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("config edit failed: {e:#}"),
        )
            .into_response();
    }
    let edited_text = doc.to_string();

    // 3. The edited file must parse and validate as a whole (independent of
    //    the scratch run above — the file may carry sections the scratch
    //    config did not).
    let parsed: walgit_config::Config = match toml::from_str(&edited_text) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("the edited config does not parse: {e}"),
            )
                .into_response();
        }
    };
    if let Err(e) = parsed.validate() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the edited config does not validate: {e:#}"),
        )
            .into_response();
    }

    // 4. Write atomically-ish (direct write; the wizard is the only writer on
    //    a single-user machine) and arm the restart when the CLI armed it.
    if let Err(e) = std::fs::write(&config_path, &edited_text) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("config file {} is not writable: {e}", config_path.display()),
        )
            .into_response();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600));
    }
    if st.setup_exit {
        // The tray watcher treats exit 75 as "restart me" (D43). The response
        // must flush first, so the exit lands a beat later.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            std::process::exit(75);
        });
    }
    tracing::info!(path = %config_path.display(), "setup wizard saved the config");
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "saved": true,
            "restart": if st.setup_exit { "supervisor" } else { "manual" },
            "warnings": warnings,
            "file": config_path.display().to_string(),
        })),
    )
        .into_response()
}

/// Edit the TOML document: `[store]` (+ `[store.s3]` / `[store.gcs]`) and,
/// for a first admin, `[server.auth]` mode + `[[server.auth.tokens]]`.
fn edit_document(
    doc: &mut toml_edit::DocumentMut,
    req: &SetupSave,
    write_admin: bool,
) -> anyhow::Result<()> {
    use toml_edit::{Item, Table, value};

    let store_entry = doc.entry("store").or_insert(Item::Table(Table::new()));
    let store_tbl = store_entry
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[store] is not a table"))?;
    store_tbl["backend"] = value(&req.store.backend);
    store_tbl["bucket"] = value(req.store.bucket.trim());
    store_tbl.remove("memory_backend_intentional");
    match req.store.backend.as_str() {
        "s3" => {
            store_tbl.remove("gcs");
            let s3_entry = store_tbl.entry("s3").or_insert(Item::Table(Table::new()));
            let s3 = s3_entry
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("[store.s3] is not a table"))?;
            s3["endpoint"] = value(req.store.endpoint.trim());
            s3["region"] = value(if req.store.region.trim().is_empty() {
                "auto"
            } else {
                req.store.region.trim()
            });
            s3["force_path_style"] = value(req.store.force_path_style);
            if req.store.access_key.trim().is_empty() {
                s3.remove("access_key");
                s3.remove("secret_key");
            } else {
                s3["access_key"] = value(req.store.access_key.trim());
                s3["secret_key"] = value(req.store.secret_key.trim());
            }
        }
        "gcs" => {
            store_tbl.remove("s3");
            let gcs_entry = store_tbl.entry("gcs").or_insert(Item::Table(Table::new()));
            let gcs = gcs_entry
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("[store.gcs] is not a table"))?;
            gcs["endpoint"] = value(req.store.endpoint.trim());
        }
        _ => {}
    }

    if write_admin && let Some(token) = req.admin_token.as_deref().filter(|t| !t.trim().is_empty())
    {
        let principal = req
            .admin_principal
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or("admin");
        let server_entry = doc.entry("server").or_insert(Item::Table(Table::new()));
        let server = server_entry
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("[server] is not a table"))?;
        let auth_entry = server.entry("auth").or_insert(Item::Table(Table::new()));
        let auth = auth_entry
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("[server.auth] is not a table"))?;
        auth["mode"] = value("token");
        let tokens_entry = auth
            .entry("tokens")
            .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
        let aot = tokens_entry
            .as_array_of_tables_mut()
            .ok_or_else(|| anyhow::anyhow!("server.auth.tokens is not an array of tables"))?;
        let mut first = Table::new();
        first.insert("principal", value(principal));
        first.insert("token", value(token.trim()));
        first.insert("write", value(true));
        first.insert("admin", value(true));
        aot.push(first);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_document_keeps_comments_and_sets_store_and_admin() {
        let text = "\
# 顶注释保留
[server]
listen = \"127.0.0.1:8081\"

[server.auth]
# 安装器初始配置:本机回环
mode = \"none\"

[store]
# 内存后端:数据不落盘
backend = \"memory\"
";
        let mut doc: toml_edit::DocumentMut = text.parse().unwrap();
        let req = SetupSave {
            store: SetupStore {
                backend: "s3".into(),
                bucket: "my-bucket".into(),
                endpoint: "https://acct.r2.cloudflarestorage.com".into(),
                region: "auto".into(),
                access_key: "AK".into(),
                secret_key: "SK".into(),
                force_path_style: true,
            },
            admin_token: Some("wgt-first-admin-token".into()),
            admin_principal: None,
        };
        edit_document(&mut doc, &req, true).unwrap();
        let out = doc.to_string();
        // Comments survive; the store keys changed; the auth gained a token.
        assert!(out.contains("# 顶注释保留"), "{out}");
        assert!(out.contains("# 内存后端:数据不落盘"), "{out}");
        assert!(out.contains("backend = \"s3\""), "{out}");
        assert!(out.contains("bucket = \"my-bucket\""), "{out}");
        assert!(
            out.contains("endpoint = \"https://acct.r2.cloudflarestorage.com\""),
            "{out}"
        );
        assert!(out.contains("access_key = \"AK\""), "{out}");
        assert!(!out.contains("mode = \"none\""), "{out}");
        assert!(out.contains("mode = \"token\""), "{out}");
        assert!(out.contains("principal = \"admin\""), "{out}");
        assert!(out.contains("admin = true"), "{out}");
        // And the whole thing still parses as a config.
        let cfg: walgit_config::Config = toml::from_str(&out).unwrap();
        assert!(matches!(cfg.store.backend, walgit_config::StoreBackend::S3));
        assert_eq!(cfg.store.s3.access_key.as_deref(), Some("AK"));
        assert_eq!(cfg.server.auth.tokens.len(), 1);
        assert!(cfg.server.auth.tokens[0].admin);
    }

    #[test]
    fn literal_creds_absent_clears_them() {
        let text = "[store]\nbackend = \"s3\"\nbucket = \"b\"\n[store.s3]\naccess_key = \"old\"\nsecret_key = \"old\"\n";
        let mut doc: toml_edit::DocumentMut = text.parse().unwrap();
        let req = SetupSave {
            store: SetupStore {
                backend: "s3".into(),
                bucket: "b".into(),
                endpoint: "http://127.0.0.1:9000".into(),
                region: "us-east-1".into(),
                access_key: String::new(),
                secret_key: String::new(),
                force_path_style: false,
            },
            admin_token: None,
            admin_principal: None,
        };
        edit_document(&mut doc, &req, true).unwrap();
        let out = doc.to_string();
        assert!(!out.contains("access_key = \"old\""), "{out}");
        let cfg: walgit_config::Config = toml::from_str(&out).unwrap();
        assert_eq!(cfg.store.s3.access_key, None);
    }
}
