//! `walgit repo` HTTP reads (issue #61): the typed host-backed commands run
//! against a live harness server — no bucket credentials on the caller side.
//! The commands print to stdout; the contract asserted here is the round
//! trip (URL building, auth header, status handling) — response shapes are
//! the server suite's own subject.

mod harness;

use std::sync::Arc;

use anyhow::Result;
use harness::{Server, TestRepo, git_in};
use walgit_cli::repo::run;
use walgit_cli::{Conn, RepoAction};

fn conn(server: &Server) -> Conn {
    Conn {
        url: Some(server.base_url.clone()),
        token: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repo_http_reads_round_trip() -> Result<()> {
    let server = Server::start().await?;
    server.put_repo("t", "cli").await?;
    let src = TestRepo::synthetic(2, 1)?;
    git_in(
        &src,
        &["remote", "add", "origin", &server.repo_url("t", "cli")],
    )?;
    git_in(&src, &["push", "-q", "origin", "main"])?;
    let conn = conn(&server);
    let cfg = Arc::new(walgit_config::Config::default());

    // refs — the cheap discovery read.
    run(
        RepoAction::Refs {
            repo: "t/cli".into(),
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;

    // resolve main → feeds commit/commits; a bogus rev must error.
    run(
        RepoAction::Resolve {
            repo: "t/cli".into(),
            rev: "main".into(),
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;
    assert!(
        run(
            RepoAction::Resolve {
                repo: "t/cli".into(),
                rev: "no-such-branch".into(),
                conn: conn.clone(),
            },
            &cfg,
        )
        .await
        .is_err(),
        "resolving a nonexistent rev is an error"
    );

    // tree / blob (JSON envelope and --raw bytes).
    run(
        RepoAction::Tree {
            repo: "t/cli".into(),
            rev: "main".into(),
            path: String::new(),
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;
    run(
        RepoAction::Blob {
            repo: "t/cli".into(),
            rev: "main".into(),
            path: "f0_0.txt".into(),
            raw: false,
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;
    run(
        RepoAction::Blob {
            repo: "t/cli".into(),
            rev: "main".into(),
            path: "f0_0.txt".into(),
            raw: true,
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;

    // commits / commit / overview / tasks.
    run(
        RepoAction::Commits {
            repo: "t/cli".into(),
            ref_: Some("main".into()),
            n: Some(5),
            skip: None,
            path: None,
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;
    let sha = git_in(&src, &["rev-parse", "main"])?.trim().to_string();
    run(
        RepoAction::Commit {
            repo: "t/cli".into(),
            sha,
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;
    run(
        RepoAction::Overview {
            repo: "t/cli".into(),
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;
    run(
        RepoAction::Tasks {
            repo: "t/cli".into(),
            follow: None,
            conn: conn.clone(),
        },
        &cfg,
    )
    .await?;

    // A nonexistent repository surfaces the HTTP status, not a hang.
    assert!(
        run(
            RepoAction::Refs {
                repo: "t/ghost".into(),
                conn,
            },
            &cfg,
        )
        .await
        .is_err(),
        "reading a nonexistent repository is an error"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repo_http_reads_carry_the_bearer_and_surface_401() -> Result<()> {
    let server = Server::start_with_tweak(|cfg| {
        cfg.server.auth.mode = walgit_config::AuthMode::Token;
        // The strict shape: anonymous reads off, so a bare request meets 401.
        cfg.server.auth.anonymous_read = false;
        cfg.server.auth.tokens = vec![walgit_config::StaticToken {
            principal: "t".into(),
            token: "s3cret".into(),
            token_env: None,
            write: true,
            admin: false,
        }];
    })
    .await?;
    // put_repo carries no credential; token mode needs the bearer on create.
    let status = reqwest::Client::new()
        .put(format!("{}/t/tok", server.base_url))
        .bearer_auth("s3cret")
        .send()
        .await?
        .status();
    assert!(status.is_success(), "repo create with bearer: {status}");
    let cfg = Arc::new(walgit_config::Config::default());

    // No credential: the server's 401 surfaces as a diagnostic error.
    let bare = conn(&server);
    let result = run(
        RepoAction::Refs {
            repo: "t/tok".into(),
            conn: bare,
        },
        &cfg,
    )
    .await;
    assert!(result.is_err(), "a token-mode host refuses bare reads");

    // With the bearer: the same read succeeds.
    let authed = Conn {
        url: Some(server.base_url.clone()),
        token: Some("s3cret".into()),
    };
    run(
        RepoAction::Refs {
            repo: "t/tok".into(),
            conn: authed,
        },
        &cfg,
    )
    .await?;
    Ok(())
}
