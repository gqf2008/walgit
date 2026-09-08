//! Per-actor collab-inbox isolation (issue #75 ⑤b, token mode): a principal
//! may push only its own `refs/collab/inbox/<actor>/*` — a second identity is
//! refused by the policy rule (the gate reads the **transport identity** the
//! token resolves to, not the entry signature), and clearing the policy rolls
//! the behavior back. Mirrors the manual matrix in issue #75 comment
//! 5548440019 as a repeatable test.

mod harness;

use harness::{Server, TestRepo, git_in};
use std::process::Command;
type TestResult = anyhow::Result<()>;

async fn start_server_with_two_principals() -> anyhow::Result<Server> {
    // A fresh loopback listener, so `token` mode validates. Two principals,
    // both with write + admin (admin is irrelevant here; the gate is policy).
    Server::start_with_tweak(|cfg| {
        cfg.server.auth.mode = walgit_config::AuthMode::Token;
        cfg.server.auth.anonymous_read = false;
        cfg.server.auth.tokens = vec![
            walgit_config::StaticToken {
                principal: "alice".into(),
                token: "alice-s3cret".into(),
                token_env: None,
                write: true,
                admin: true,
            },
            walgit_config::StaticToken {
                principal: "bob".into(),
                token: "bob-s3cret".into(),
                token_env: None,
                write: true,
                admin: true,
            },
        ];
    })
    .await
}

fn inbox_policy() -> String {
    r#"{
  "version": 1,
  "rules": [
    {
      "name": "lock-alice-inbox",
      "match": { "refs": ["refs/collab/inbox/alice/*"] },
      "effect": {
        "protect": { "restricts": ["create", "update", "delete"], "bypass": ["alice"] }
      }
    },
    {
      "name": "lock-bob-inbox",
      "match": { "refs": ["refs/collab/inbox/bob/*"] },
      "effect": { "protect": { "restricts": ["create", "update", "delete"], "bypass": ["bob"] }
      }
    }
  ]
}"#
    .to_string()
}

/// Redact everything in `s` that could carry this test's credential.
///
/// Two vectors, both real in git's stderr: the **URL userinfo** form git echoes back on a
/// transport failure (`unable to access 'https://git:<token>@host/…'`), and any
/// **`Authorization:` header line** (what `GIT_TRACE_CURL` used to print here — including the
/// base64 form, which no literal-token replacement can catch, so the whole line goes). The
/// returned string is what feeds `assert!(…, "{err}")` messages, and those land in CI logs on
/// failure (issue #142). The token here is a fixture, not a secret — the point is that the
/// shape never becomes how this suite reports failures.
fn scrub_credentials(s: &str, token: &str) -> String {
    let replaced = s.replace(token, "<redacted>");
    replaced
        .lines()
        .filter(|l| !l.to_ascii_lowercase().contains("authorization:"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Push `HEAD` as `refs/collab/inbox/<actor>/<name>` from `src` with the
/// transport identity of `token`. Returns success (the server accepted) and
/// **credential-scrubbed** stderr ([`scrub_credentials`]).
fn push_inbox(src: &TestRepo, server: &Server, actor: &str, name: &str, token: &str) -> (bool, String) {
    let ref_name = format!("refs/collab/inbox/{actor}/{name}");
    // An orphan commit per push: the inbox is append-only refs, unrelated
    // history between them is the normal shape.
    let base = format!("work-{actor}-{name}");
    let mut ok = git_in(src, &["checkout", "--orphan", &base]).is_ok();
    ok &= git_in(
        src,
        &[
            "-c",
            "user.name=tester",
            "-c",
            "user.email=t@walgit",
            "commit",
            "--allow-empty",
            "-m",
            name,
        ],
    )
    .is_ok();
    if !ok {
        return (false, String::new());
    }
    // URL userinfo form — the #79 challenge flow (§1.3); password = token.
    // No GIT_TRACE_CURL here: it printed the full request-header set (including
    // `Authorization: Basic <base64(git:token)>`) into the stderr that feeds every
    // assert message in this file (issue #142).
    let out = Command::new("git")
        .current_dir(&src.dir)
        .args([
            "push",
            &server.repo_url_with_userinfo("t", "secured", "git", token),
            &format!("HEAD:{ref_name}"),
        ])
        .output();
    match out {
        Ok(o) => (
            o.status.success(),
            scrub_credentials(&String::from_utf8_lossy(&o.stderr), token),
        ),
        Err(e) => (false, format!("{e}")),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn collab_inbox_pushes_are_gated_per_actor_in_token_mode() -> TestResult {
    let server = start_server_with_two_principals().await?;
    // put_repo is bearer-less; token mode needs the credential on create, so
    // the repo is created through the API with alice's token.
    let status = reqwest::Client::new()
        .put(format!("{}/t/secured", server.base_url))
        .bearer_auth("alice-s3cret")
        .send()
        .await?
        .status();
    assert!(status.is_success(), "repo create with bearer: {status}");

    let put = reqwest::Client::new()
        .put(format!(
            "{}/t/secured/policy",
            server.base_url
        ))
        .bearer_auth("alice-s3cret")
        .header("content-type", "application/json")
        .body(inbox_policy())
        .send()
        .await?;
    assert_eq!(put.status(), 204, "{}", put.text().await?);

    let src = TestRepo::synthetic(1, 1)?;

    // alice writes alice's inbox: allowed.
    let (ok, err) = push_inbox(&src, &server, "alice", "e1", "alice-s3cret");
    assert!(ok, "alice pushing her own inbox failed: {err}");

    // bob pushes alice's inbox: refused by the rule (transport identity wins
    // over any signature the payload may carry).
    let (ok, err) = push_inbox(&src, &server, "alice", "e2", "bob-s3cret");
    assert!(
        !ok,
        "bob pushing alice's inbox must be rejected; stderr: {err}"
    );
    assert!(
        err.contains("lock-alice-inbox") || err.contains("rejected by rule"),
        "stderr should name the rule: {err}"
    );

    // bob writes bob's own inbox: allowed.
    let (ok, err) = push_inbox(&src, &server, "bob", "e3", "bob-s3cret");
    assert!(ok, "bob pushing his own inbox failed: {err}");

    // Clearing the policy rolls the behavior back: bob can now push alice's
    // inbox (the read-side signature check remains the second layer).
    let cleared = reqwest::Client::new()
        .delete(format!(
            "{}/t/secured/policy",
            server.base_url
        ))
        .bearer_auth("bob-s3cret")
        .send()
        .await?;
    assert_eq!(cleared.status(), 204);
    let (ok, err) = push_inbox(&src, &server, "alice", "e4", "bob-s3cret");
    assert!(
        ok,
        "after policy clear the same push must succeed; stderr: {err}"
    );
    Ok(())
}

/// The credential this suite pushes with must not survive into the text that feeds
/// `assert!(…, "{err}")` — those messages are printed into CI logs on failure.
/// Covers both vectors [`scrub_credentials`] names: the URL userinfo git echoes on a
/// transport failure, and an `Authorization:` header line whose base64 payload no
/// literal-token replacement could catch (issue #142).
#[test]
fn failure_text_never_carries_the_credential() {
    let token = "alice-s3cret";
    let git_stderr = "\
fatal: unable to access 'https://git:alice-s3cret@127.0.0.1:4321/t/secured/': Failed to connect
=> Send header: POST /t/secured.git/info/refs?service=git-receive-pack HTTP/1.1
=> Send header: Authorization: Basic Z2l0OmFsaWNlLXMzY3JldA==
=> Send header: User-Agent: git/2.50.1
remote: error: deny-rule: lock-alice-inbox
";
    let scrubbed = scrub_credentials(git_stderr, token);
    assert!(
        !scrubbed.contains(token),
        "the literal token survived the scrub: {scrubbed}"
    );
    assert!(
        !scrubbed.to_ascii_lowercase().contains("authorization:"),
        "an Authorization header line survived the scrub: {scrubbed}"
    );
    assert!(
        !scrubbed.contains("Z2l0OmFsaWNlLXMzY3Jld"),
        "the base64 credential survived the scrub: {scrubbed}"
    );
    // The scrub must not cost the diagnosis: what a failing push is about still reads out.
    assert!(
        scrubbed.contains("deny-rule: lock-alice-inbox") && scrubbed.contains("Failed to connect"),
        "scrubbing removed more than credentials: {scrubbed}"
    );
}
