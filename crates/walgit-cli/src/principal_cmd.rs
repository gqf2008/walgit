//! `walgit principal` — host-global principal registry commands (issue #76):
//! register / list / revoke / rotate against `/api/v1/principals` (D1 §5
//! cross-repo extension). HTTP-only, no bucket access.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use clap::Subcommand;
use serde_json::json;

use crate::collab_cmd::read_signing_key;

#[derive(Subcommand)]
pub enum PrincipalAction {
    /// Register (or re-register = rotate) the caller's public key in the host
    /// registry: one registration verifies in every repository of that host.
    Register {
        /// Host root URL, e.g. `http://walgit.localhost:8081`.
        #[arg(long)]
        url: String,
        /// Principal (refname-safe); must equal the authenticated principal.
        #[arg(long)]
        principal: String,
        /// Ed25519 signing key: 32 raw bytes as hex (same format as `collab`).
        #[arg(long)]
        key: PathBuf,
        /// Bearer token (default `$WALGIT_TOKEN`).
        #[arg(long)]
        token: Option<String>,
    },
    /// Rotate = re-register with a new key (overwrites; revocation history is
    /// not kept by the registry, only by the signed trail it verified).
    Rotate {
        #[arg(long)]
        url: String,
        #[arg(long)]
        principal: String,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        token: Option<String>,
    },
    /// List every host-registered principal (`principal -> public_key`).
    List {
        #[arg(long)]
        url: String,
        #[arg(long)]
        token: Option<String>,
    },
    /// Revoke the caller's own principal (DELETE); self only for now.
    Revoke {
        #[arg(long)]
        url: String,
        #[arg(long)]
        principal: String,
        #[arg(long)]
        token: Option<String>,
    },
}

pub fn run(action: PrincipalAction) -> Result<()> {
    match action {
        PrincipalAction::Register {
            url,
            principal,
            key,
            token,
        } => put(&url, &principal, &key, token.as_deref(), "registered"),
        PrincipalAction::Rotate {
            url,
            principal,
            key,
            token,
        } => put(&url, &principal, &key, token.as_deref(), "rotated"),
        PrincipalAction::List { url, token } => list(&url, token.as_deref()),
        PrincipalAction::Revoke {
            url,
            principal,
            token,
        } => revoke(&url, &principal, token.as_deref()),
    }
}

fn bearer(token: Option<&str>) -> Result<String> {
    let t = token
        .map(str::to_string)
        .or_else(|| std::env::var("WALGIT_TOKEN").ok())
        .filter(|t| !t.trim().is_empty())
        .with_context(|| "no bearer token: pass --token or set WALGIT_TOKEN")?;
    Ok(t)
}

fn put(url: &str, principal: &str, key: &PathBuf, token: Option<&str>, verb: &str) -> Result<()> {
    if !refname_safe(principal) {
        bail!("principal {principal:?} is not a refname-safe segment");
    }
    let token = bearer(token)?;
    let sk = read_signing_key(key)?;
    let public_key = base64::engine::general_purpose::STANDARD.encode(sk.verifying_key().to_bytes());
    let body = json!({ "public_key": public_key });
    tokio::runtime::Handle::current().block_on(async move {
        let resp = reqwest::Client::new()
            .put(format!("{url}/api/v1/principals/{principal}"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .context("PUT host principal")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::ensure!(
            status.is_success(),
            "PUT {url}/api/v1/principals/{principal} -> {status}: {text}"
        );
        Ok::<_, anyhow::Error>(())
    })?;
    println!("{verb} {principal} at {url}");
    Ok(())
}

fn list(url: &str, token: Option<&str>) -> Result<()> {
    let token = bearer(token)?;
    let out = tokio::runtime::Handle::current().block_on(async move {
        let resp = reqwest::Client::new()
            .get(format!("{url}/api/v1/principals"))
            .bearer_auth(token)
            .send()
            .await
            .context("GET host principals")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::ensure!(
            status.is_success(),
            "GET {url}/api/v1/principals -> {status}: {text}"
        );
        Ok::<_, anyhow::Error>(text)
    })?;
    println!("{out}");
    Ok(())
}

fn revoke(url: &str, principal: &str, token: Option<&str>) -> Result<()> {
    let token = bearer(token)?;
    tokio::runtime::Handle::current().block_on(async move {
        let resp = reqwest::Client::new()
            .delete(format!("{url}/api/v1/principals/{principal}"))
            .bearer_auth(token)
            .send()
            .await
            .context("DELETE host principal")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::ensure!(
            status.is_success(),
            "DELETE {url}/api/v1/principals/{principal} -> {status}: {text}"
        );
        Ok::<_, anyhow::Error>(())
    })?;
    println!("revoked {principal} at {url}");
    Ok(())
}

/// D1 principal syntax: `[A-Za-z0-9][A-Za-z0-9._@-]*`.
fn refname_safe(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-'))
}
