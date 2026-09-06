//! Receive-pack forwarding to a push broker (`wal.push_broker_url`): one warm
//! writer that batches the manifest CAS for many small repositories.
//!
//! The request body and broker response are deliberately kept as streams.  A
//! front falls back to its local receive-pack path only when no broker response
//! was obtained, or when the broker explicitly reports a gateway-unavailable
//! status before a response body is consumed.
//!
//! The hop authenticates with `wal.push_broker_token` (or `WALGIT_BROKER_TOKEN`):
//! a static token the broker lists under `server.auth.tokens` with `write = true`
//! and whose principal is in its `trusted_forwarders`, so the end user's identity
//! travels in `X-Walgit-Principal`. A loopback broker needs no token.

use std::time::Instant;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use futures::StreamExt;

use crate::{auth::Principal, repo::RepoRoute};

/// The result of trying to forward one receive-pack body.
pub enum ForwardOutcome {
    /// A broker response was obtained and can be returned to the Git client.
    Response(Response),
    /// The broker was not reached or returned a gateway-unavailable response.
    /// The caller still owns the body only when the broker was not reached before
    /// any bytes were sent; smart.rs therefore buffers only the local fallback
    /// path and never retries after an acknowledged broker request.
    Fallback,
}

/// Stream one receive-pack request to the broker and stream its response back.
/// `X-Walgit-Forwarded: 1` is added by the caller and must not be forwarded again.
#[tracing::instrument(name = "push.forward", skip_all)]
pub async fn receive_pack(
    broker_url: &str,
    route: &RepoRoute,
    headers: &HeaderMap,
    body: Body,
    principal: &Principal,
    broker_token: Option<&str>,
) -> ForwardOutcome {
    let started = Instant::now();
    let endpoint = format!(
        "{}/{}/{}.git/git-receive-pack",
        broker_url.trim_end_matches('/'),
        route.id.owner(),
        route.id.name()
    );
    let client = reqwest::Client::new();
    let stream = body.into_data_stream().map(|chunk| {
        chunk.map_err(|e| std::io::Error::other(e.to_string()))
    });
    let mut request = client
        .post(&endpoint)
        .body(reqwest::Body::wrap_stream(stream));

    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_ENCODING,
        header::EXPECT,
        header::CONTENT_RANGE,
        header::HeaderName::from_static("git-protocol"),
        header::HeaderName::from_static("x-request-id"),
    ] {
        if let Some(value) = headers.get(&name) {
            request = request.header(name, value);
        }
    }
    request = request.header("X-Walgit-Forwarded", "1");
    if let Ok(value) = HeaderValue::try_from(principal.name.as_str()) {
        request = request.header("X-Walgit-Principal", value);
    }

    if !is_local_broker(broker_url) {
        let token = std::env::var("WALGIT_BROKER_TOKEN")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| broker_token.map(str::to_string).filter(|v| !v.is_empty()));
        if let Some(token) = token { request = request.bearer_auth(token) } else {
            tracing::warn!(
                elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "push broker token unset (wal.push_broker_token / WALGIT_BROKER_TOKEN); falling back"
            );
            return ForwardOutcome::Fallback;
        }
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX), "push broker unavailable; falling back");
            return ForwardOutcome::Fallback;
        }
    };
    if matches!(
        response.status(),
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT
    ) {
        tracing::warn!(
            status = response.status().as_u16(),
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "push broker gateway failure; falling back"
        );
        return ForwardOutcome::Fallback;
    }
    // A 401 from the broker is the *hop's* credential failing (rotated
    // `wal.push_broker_token`), not the end user's — but relayed bare it
    // makes git call `erase` on its helpers and drop the user's good token,
    // and a 401 is not in retryable-auth territory for libcurl mid-request.
    // The broker is an optimisation, never a dependency (D28): fall back to
    // the local receive-pack path instead of surfacing the broker's
    // challenge as ours (issue #92).
    if response.status() == StatusCode::UNAUTHORIZED {
        tracing::warn!(
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "push broker rejected the hop credential (401); falling back to local receive-pack"
        );
        return ForwardOutcome::Fallback;
    }

    let status = response.status();
    let response_headers = response.headers().clone();
    let stream = response.bytes_stream().map(|chunk| {
        chunk.map_err(|e| std::io::Error::other(e.to_string()))
    });
    let mut builder = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_ENCODING,
        header::CACHE_CONTROL,
        header::ETAG,
        // The broker's auth verdict travels with its challenge: a relayed 401/403
        // without `WWW-Authenticate` is a bare failure git cannot act on (and
        // `erase`s a good credential for) — relay the header verbatim (issue #92).
        header::WWW_AUTHENTICATE,
    ] {
        if let Some(value) = response_headers.get(&name) {
            builder = builder.header(name, value);
        }
    }
    let output = builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| Response::new(Body::empty()));
    let outcome = if status.is_success() { "ok" } else { "error" };
    metrics::counter!("walgit_push_forwarded_total", "outcome" => outcome).increment(1);
    tracing::info!(
        status = status.as_u16(),
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "push broker response streamed"
    );
    ForwardOutcome::Response(output)
}

fn is_local_broker(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A one-shot broker: reads exactly one request (headers + body — either a
    /// satisfied Content-Length or a chunked terminator / EOF), answers `status`
    /// plus `www` when given, closes.
    async fn mini_broker(status: StatusCode, www: Option<&str>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let www = www.map(str::to_string);
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = sock.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                let Some((head, rest)) = text.split_once("\r\n\r\n") else {
                    continue;
                };
                let content_length = head.lines().find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().to_string())
                });
                let done = match content_length.as_deref() {
                    Some(len) => rest.len() >= len.parse::<usize>().unwrap_or(0),
                    None => rest.ends_with("0\r\n\r\n"),
                };
                if done {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 {} {}\r\nContent-Length: 0\r\n{}Connection: close\r\n\r\n",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
                www.map(|w| format!("WWW-Authenticate: {w}\r\n"))
                    .unwrap_or_default(),
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            let _ = sock.shutdown().await;
        });
        format!("http://{addr}")
    }

    async fn forward_to(broker_url: &str) -> ForwardOutcome {
        let route = RepoRoute {
            id: "o/r".parse().unwrap(),
            subpath: "git-receive-pack".into(),
            had_git_suffix: true,
        };
        let principal = Principal {
            name: "dev@example.com".into(),
            write: true,
            admin: false,
            anonymous: false,
        };
        receive_pack(
            broker_url,
            &route,
            &HeaderMap::new(),
            Body::empty(),
            &principal,
            None,
        )
        .await
    }

    /// Issue #92: a broker 401 is the *hop's* credential failing, not the end
    /// user's — relaying it bare makes git `erase` the user's good token from
    /// its helpers. The broker is an optimisation (D28): fall back to the local
    /// receive-pack path.
    #[tokio::test]
    async fn broker_401_falls_back_instead_of_relaying() {
        let broker = mini_broker(StatusCode::UNAUTHORIZED, Some("Basic realm=\"walgit\"")).await;
        let outcome = forward_to(&broker).await;
        assert!(
            matches!(outcome, ForwardOutcome::Fallback),
            "a broker 401 must fall back to the local path (D28)"
        );
    }

    /// Issue #92: a relayed 4xx keeps the broker's challenge — a bare 401/403
    /// is a failure git cannot act on.
    #[tokio::test]
    async fn relayed_4xx_carries_www_authenticate() {
        let broker = mini_broker(
            StatusCode::FORBIDDEN,
            Some("Bearer realm=\"walgit\", error=\"insufficient_scope\""),
        )
        .await;
        let outcome = forward_to(&broker).await;
        let ForwardOutcome::Response(resp) = outcome else {
            panic!("a relayed 403 must surface, not fall back");
        };
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            resp.headers()
                .get(header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default(),
            "Bearer realm=\"walgit\", error=\"insufficient_scope\"",
            "the broker's WWW-Authenticate must travel with the relayed response"
        );
    }
}
