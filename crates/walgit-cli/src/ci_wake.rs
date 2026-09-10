//! D1-CI §4.2 (issue #161): the hosted runner's wake endpoint — an
//! events-bridge (D32, docs/EVENTS.md) consumer. A verified batch of ref
//! events is a *hint* to run one evaluation pass now; the trigger truth
//! stays the ls-remote diff (§4), so a spoofed, lost or duplicated wake can
//! only ever cause a no-op pass. The webhook secret lives only in the
//! client's env/argv — never in a bucket object, a ref, or a result entry
//! (§9 red line).

use std::collections::VecDeque;
use std::sync::{Arc, mpsc};

use anyhow::{Context, Result};
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, StatusCode};
use parking_lot::Mutex;

/// Delivery-id dedup ring depth (EVENTS.md: at-least-once whole batches).
const DELIVERY_RING: usize = 256;

/// A batch body is a small JSON array; anything bigger is refused before
/// parsing (the bridge sends ref events, not payloads).
const BATCH_MAX_BYTES: usize = 1 << 20;

/// What one `POSTed` batch decides (the handler maps these to status codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeVerdict {
    /// At least one triggerable ref event (§4: `refs/heads/*`, `refs/tags/*`)
    /// — evaluate now.
    Wake,
    /// Well-formed batch, nothing on the trigger surface (`refs/collab/*`,
    /// `refs/pull/*`, …) — acked, no pass.
    Ignored,
    /// `X-Walgit-Delivery` already seen — the bridge's at-least-once retry;
    /// acked, no pass.
    Duplicate,
    /// A secret is configured and the signature is missing or wrong — the
    /// batch is rejected *before parsing* (EVENTS.md's consumer contract).
    Unauthorized,
    /// Oversized or not a JSON array of events.
    Malformed,
}

/// Shared wake state: the secret (client-side only) and the delivery ring.
pub struct WakeState {
    secret: Option<Vec<u8>>,
    seen: Mutex<VecDeque<String>>,
}

impl WakeState {
    pub fn new(secret: Option<Vec<u8>>) -> Self {
        WakeState {
            secret,
            seen: Mutex::new(VecDeque::new()),
        }
    }

    /// verify → dedup → parse → filter — EVENTS.md's consumer order: the
    /// signature (when a secret is configured) is checked with a
    /// constant-time compare *before* the body is parsed.
    pub fn evaluate(
        &self,
        signature: Option<&str>,
        delivery: Option<&str>,
        body: &[u8],
    ) -> WakeVerdict {
        if body.len() > BATCH_MAX_BYTES {
            return WakeVerdict::Malformed;
        }
        if let Some(secret) = &self.secret {
            let Some(sig) = signature else {
                return WakeVerdict::Unauthorized;
            };
            if !signature_matches(secret, body, sig) {
                return WakeVerdict::Unauthorized;
            }
        }
        if let Some(d) = delivery {
            let mut seen = self.seen.lock();
            if seen.iter().any(|x| x == d) {
                return WakeVerdict::Duplicate;
            }
            if seen.len() == DELIVERY_RING {
                seen.pop_front();
            }
            seen.push_back(d.to_string());
        }
        triggerable(body)
    }
}

/// `sha256=<hex HMAC-SHA256(body, secret)>`, compared in constant time by
/// `Mac::verify_slice`; any shape deviation fails closed.
fn signature_matches(secret: &[u8], body: &[u8], header: &str) -> bool {
    use hmac::{Hmac, Mac};
    let Some(given) = header
        .strip_prefix("sha256=")
        .and_then(|h| hex::decode(h).ok())
    else {
        return false;
    };
    // Hmac<Sha256> accepts any key length: new_from_slice cannot fail.
    let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&given).is_ok()
}

/// The §4 trigger surface: any event for `refs/heads/*` or `refs/tags/*`.
fn triggerable(body: &[u8]) -> WakeVerdict {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return WakeVerdict::Malformed;
    };
    let Some(arr) = v.as_array() else {
        return WakeVerdict::Malformed;
    };
    let hit = arr
        .iter()
        .filter_map(|e| e.get("ref_name").and_then(serde_json::Value::as_str))
        .any(|r| r.starts_with("refs/heads/") || r.starts_with("refs/tags/"));
    if hit { WakeVerdict::Wake } else { WakeVerdict::Ignored }
}

/// Bind (in the caller, so a bind failure is immediate and the bound port is
/// known) and serve the wake endpoint on a dedicated thread with a
/// current-thread runtime — the runner's poll loop stays synchronous. Each
/// `Wake` `try_send`s into a cap-1 channel: a second wake while one is
/// pending is dropped — coalescing, the same philosophy as §4.1's poll.
///
/// The receiver is what the poll loop waits on between passes: a wake hint
/// cuts the `--interval` nap short.
pub fn spawn_wake_listener(
    addr: std::net::SocketAddr,
    secret: Option<Vec<u8>>,
) -> Result<(std::net::SocketAddr, mpsc::Receiver<()>)> {
    let std_listener = std::net::TcpListener::bind(addr)
        .with_context(|| format!("bind wake listener {addr}"))?;
    std_listener
        .set_nonblocking(true)
        .with_context(|| format!("set wake listener {addr} nonblocking"))?;
    let bound = std_listener.local_addr()?;
    let (tx, rx) = mpsc::sync_channel::<()>(1);
    let state = Arc::new(WakeState::new(secret));
    std::thread::Builder::new()
        .name("ci-wake".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("ci: wake listener runtime: {e}");
                    return;
                }
            };
            rt.block_on(async move {
                let Ok(listener) = tokio::net::TcpListener::from_std(std_listener) else {
                    eprintln!("ci: wake listener: from_std failed");
                    return;
                };
                if let Err(e) = axum::serve(listener, wake_router(state, tx)).await {
                    eprintln!("ci: wake listener stopped: {e}");
                }
            });
        })
        .context("spawn ci-wake thread")?;
    Ok((bound, rx))
}

/// POST on any path (the bridge POSTs to `events.webhook_url` verbatim; the
/// path is the deployer's choice). Other methods 405.
fn wake_router(state: Arc<WakeState>, tx: mpsc::SyncSender<()>) -> axum::Router {
    axum::Router::new()
        .fallback(axum::routing::post(
            move |headers: HeaderMap, body: axum::body::Bytes| {
                let state = Arc::clone(&state);
                let tx = tx.clone();
                async move {
                    let signature = headers
                        .get("x-walgit-signature")
                        .and_then(|v| v.to_str().ok());
                    let delivery = headers
                        .get("x-walgit-delivery")
                        .and_then(|v| v.to_str().ok());
                    match state.evaluate(signature, delivery, &body) {
                        WakeVerdict::Wake => {
                            // Full channel = a wake is already pending: this
                            // hint folds into it (coalesce).
                            let _ = tx.try_send(());
                            StatusCode::OK
                        }
                        WakeVerdict::Ignored | WakeVerdict::Duplicate => StatusCode::OK,
                        WakeVerdict::Unauthorized => StatusCode::UNAUTHORIZED,
                        WakeVerdict::Malformed => StatusCode::BAD_REQUEST,
                    }
                }
            },
        ))
        .layer(DefaultBodyLimit::max(BATCH_MAX_BYTES))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact construction the bridge's `WebhookSink::signature` uses.
    fn signed(secret: &[u8], body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    const BRANCH: &str = r#"[{"repo":"o/r","ref_name":"refs/heads/main","old":"11","new":"22","_walgit":{"seq":"7"}}]"#;
    const TAG: &str = r#"[{"repo":"o/r","ref_name":"refs/tags/v1","old":"00","new":"22","_walgit":{"seq":"8"}}]"#;
    const COLLAB: &str = r#"[{"repo":"o/r","ref_name":"refs/collab/inbox/alice/1","old":"00","new":"22","_walgit":{"seq":"9"}}]"#;

    #[test]
    fn a_verified_batch_with_a_trigger_ref_wakes() {
        let st = WakeState::new(Some(b"s3cret".to_vec()));
        let sig = signed(b"s3cret", BRANCH.as_bytes());
        assert_eq!(
            st.evaluate(Some(&sig), Some("d1"), BRANCH.as_bytes()),
            WakeVerdict::Wake
        );
        let sig = signed(b"s3cret", TAG.as_bytes());
        assert_eq!(
            st.evaluate(Some(&sig), Some("d2"), TAG.as_bytes()),
            WakeVerdict::Wake
        );
    }

    #[test]
    fn a_wrong_or_missing_signature_is_rejected_before_parsing() {
        let st = WakeState::new(Some(b"s3cret".to_vec()));
        // Missing entirely.
        assert_eq!(
            st.evaluate(None, Some("d1"), BRANCH.as_bytes()),
            WakeVerdict::Unauthorized
        );
        // Wrong key.
        let bad = signed(b"other", BRANCH.as_bytes());
        assert_eq!(
            st.evaluate(Some(&bad), Some("d2"), BRANCH.as_bytes()),
            WakeVerdict::Unauthorized
        );
        // Tampered body under its own untampered signature.
        let sig = signed(b"s3cret", BRANCH.as_bytes());
        let tampered = BRANCH.replace("main", "main2");
        assert_eq!(
            st.evaluate(Some(&sig), Some("d3"), tampered.as_bytes()),
            WakeVerdict::Unauthorized
        );
        // Garbage shape (`sha1=…`, bare hex, truncated) fails closed.
        for bad in ["sha1=abc", "abc", "sha256=zz", "sha256="] {
            assert_eq!(
                st.evaluate(Some(bad), Some("d4"), BRANCH.as_bytes()),
                WakeVerdict::Unauthorized,
                "{bad}"
            );
        }
        // Proof the check is before parsing: a body that is not JSON at all
        // is still Unauthorized (not Malformed) without a valid signature.
        assert_eq!(
            st.evaluate(None, Some("d5"), b"this is not json"),
            WakeVerdict::Unauthorized
        );
    }

    #[test]
    fn a_duplicate_delivery_is_acked_without_waking() {
        let st = WakeState::new(Some(b"s3cret".to_vec()));
        let sig = signed(b"s3cret", BRANCH.as_bytes());
        assert_eq!(
            st.evaluate(Some(&sig), Some("delivery-1"), BRANCH.as_bytes()),
            WakeVerdict::Wake
        );
        // The bridge's at-least-once retry: same X-Walgit-Delivery.
        assert_eq!(
            st.evaluate(Some(&sig), Some("delivery-1"), BRANCH.as_bytes()),
            WakeVerdict::Duplicate
        );
        // A different batch id wakes again.
        assert_eq!(
            st.evaluate(Some(&sig), Some("delivery-2"), BRANCH.as_bytes()),
            WakeVerdict::Wake
        );
    }

    #[test]
    fn non_trigger_refs_are_ignored_and_garbage_is_malformed() {
        let st = WakeState::new(Some(b"s3cret".to_vec()));
        let sig = signed(b"s3cret", COLLAB.as_bytes());
        assert_eq!(
            st.evaluate(Some(&sig), Some("d1"), COLLAB.as_bytes()),
            WakeVerdict::Ignored
        );
        let sig = signed(b"s3cret", b"[]");
        assert_eq!(
            st.evaluate(Some(&sig), Some("d2"), b"[]"),
            WakeVerdict::Ignored
        );
        for (bad, want) in [
            (&b"{}".as_slice(), WakeVerdict::Malformed),
            (&b"not json".as_slice(), WakeVerdict::Malformed),
            (&b"[1,2]".as_slice(), WakeVerdict::Ignored),
        ] {
            let sig = signed(b"s3cret", bad);
            assert_eq!(st.evaluate(Some(&sig), None, bad), want, "{bad:?}");
        }
    }

    #[test]
    fn without_a_secret_unsigned_batches_are_hints_only() {
        // Documented (§4.2): with no secret configured, anyone who can reach
        // the port can cause… one no-op evaluation pass. The trigger truth
        // stays the ls-remote diff, so this is availability-neutral.
        let st = WakeState::new(None);
        assert_eq!(
            st.evaluate(None, Some("d1"), BRANCH.as_bytes()),
            WakeVerdict::Wake
        );
        // …but dedup still applies.
        assert_eq!(
            st.evaluate(None, Some("d1"), BRANCH.as_bytes()),
            WakeVerdict::Duplicate
        );
    }

    #[test]
    fn the_delivery_ring_is_bounded() {
        let st = WakeState::new(None);
        for i in 0..super::DELIVERY_RING * 2 {
            let d = format!("d{i}");
            assert_eq!(st.evaluate(None, Some(&d), BRANCH.as_bytes()), WakeVerdict::Wake);
        }
        assert_eq!(st.seen.lock().len(), super::DELIVERY_RING);
        // The oldest ids fell out of the ring; a recent one is still deduped.
        let recent = format!("d{}", super::DELIVERY_RING * 2 - 1);
        assert_eq!(
            st.evaluate(None, Some(&recent), BRANCH.as_bytes()),
            WakeVerdict::Duplicate
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn listener_verifies_signature_and_sends_a_wake() {
        let secret = b"s3cret";
        let (addr, rx) = spawn_wake_listener(
            "127.0.0.1:0".parse().expect("valid bind address"),
            Some(secret.to_vec()),
        )
        .expect("listener binds");
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/walgit");

        let response = client
            .post(&url)
            .header("x-walgit-signature", signed(secret, BRANCH.as_bytes()))
            .header("x-walgit-delivery", "d1")
            .body(BRANCH)
            .send()
            .await
            .expect("POST reaches the listener");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        rx.recv_timeout(std::time::Duration::from_secs(1))
            .expect("verified trigger event wakes the runner");

        let response = client
            .post(&url)
            .header("x-walgit-signature", signed(b"wrong", BRANCH.as_bytes()))
            .header("x-walgit-delivery", "d2")
            .body(BRANCH)
            .send()
            .await
            .expect("POST reaches the listener");
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert!(rx.try_recv().is_err(), "an unauthenticated batch cannot wake");

        let response = client
            .post(&url)
            .header("x-walgit-signature", signed(secret, COLLAB.as_bytes()))
            .header("x-walgit-delivery", "d3")
            .body(COLLAB)
            .send()
            .await
            .expect("POST reaches the listener");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(rx.try_recv().is_err(), "a non-trigger ref does not wake");
    }
}
