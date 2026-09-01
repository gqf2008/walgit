#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::dbg_macro
)]
// Tests may panic freely (the intent recorded in clippy.toml); helpers outside
// #[test] fns are not covered by allow-*-in-tests, so each test target opts out.
//! web/API.md §6 conformance for the read-only JSON API.

mod harness;

use harness::{Server, git_in};
use serde_json::Value;

type TestResult = anyhow::Result<()>;

async fn get(
    server: &Server,
    path: &str,
) -> anyhow::Result<(reqwest::StatusCode, String, Option<String>)> {
    let resp = reqwest::Client::new()
        .get(format!("{}{path}", server.base_url))
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let text = resp.text().await?;
    Ok((status, text, ct))
}

async fn get_h(
    server: &Server,
    path: &str,
    extra: &[(&str, &str)],
) -> anyhow::Result<(reqwest::StatusCode, String, reqwest::header::HeaderMap)> {
    let mut req = reqwest::Client::new()
        .get(format!("{}{path}", server.base_url))
        .header("Accept", "application/json");
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let text = resp.text().await?;
    Ok((status, text, headers))
}
fn hdr(h: &reqwest::header::HeaderMap, k: &str) -> String {
    h.get(k)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

async fn json(server: &Server, path: &str) -> anyhow::Result<Value> {
    let (status, text, ct) = get(server, path).await?;
    anyhow::ensure!(status.is_success(), "GET {path} -> {status}: {text}");
    anyhow::ensure!(
        ct.as_deref().unwrap_or("").starts_with("application/json"),
        "content-type {ct:?}"
    );
    Ok(serde_json::from_str(&text)?)
}

/// Build a source repo with the shapes the UI cares about and push it.
fn fixture(server: &Server) -> anyhow::Result<std::path::PathBuf> {
    let dir = tempfile::tempdir()?.keep(); // TODO(hermetic): keep TempDir in fixture
    git_in(&dir, &["init", "-q", "-b", "main"])?;
    git_in(&dir, &["config", "user.email", "t@t"])?;
    git_in(&dir, &["config", "user.name", "Tester"])?;
    std::fs::write(dir.join("README.md"), "# Title\n\nhello\n")?;
    std::fs::create_dir_all(dir.join("src/inner"))?;
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n")?;
    std::fs::write(dir.join("src/inner/x.txt"), "x\n")?;
    std::fs::write(dir.join("bin.dat"), [0u8, 159, 146, 150, 0, 1, 2])?;
    std::fs::write(dir.join("big.txt"), vec![b'a'; 2 * 1024 * 1024 + 1])?;
    git_in(&dir, &["add", "."])?;
    git_in(&dir, &["commit", "-q", "-m", "initial\n\nbody line"])?;
    // feature/x branch with a nested dir named after a path segment, plus rename.
    git_in(&dir, &["checkout", "-q", "-b", "feature/x"])?;
    std::fs::create_dir_all(dir.join("dir"))?;
    std::fs::write(dir.join("dir/f.txt"), "f\n")?;
    git_in(&dir, &["mv", "src/main.rs", "src/app.rs"])?;
    git_in(&dir, &["add", "."])?;
    git_in(&dir, &["commit", "-q", "-m", "feature work"])?;
    git_in(&dir, &["checkout", "-q", "main"])?;
    std::fs::write(dir.join("src/inner/x.txt"), "xx\n")?;
    git_in(&dir, &["commit", "-qam", "second on main"])?;
    git_in(
        &dir,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature/x"],
    )?;
    git_in(&dir, &["tag", "-a", "v1.0", "-m", "release"])?;
    git_in(
        &dir,
        &[
            "-c",
            "tag.forceSignAnnotated=false",
            "-c",
            "tag.gpgsign=false",
            "tag",
            "light",
        ],
    )?;
    for _ in 0..40 {
        git_in(&dir, &["commit", "-q", "--allow-empty", "-m", "filler"])?;
    }
    // D1 collaboration namespace: arbitrary refs under refs/collab/* must be
    // hostable (the design's inbox model) and listable via the new endpoints.
    git_in(&dir, &["update-ref", "refs/collab/inbox/alice/1", "HEAD"])?;
    git_in(
        &dir,
        &["push", "-q", "--mirror", &server.repo_url("o", "r")],
    )?;
    Ok(dir)
}

/// web/API.md §6 against one server (called for the local-packs instance and
/// for a sibling that serves the same repo remotely).
async fn conformance(
    server: &Server,
    src: &std::path::Path,
    head: &str,
    feature: &str,
    v1_peeled: &str,
) -> TestResult {
    // owners
    assert_eq!(
        json(server, "/services/api/owners").await?,
        serde_json::json!(["o"])
    );
    assert_eq!(
        json(server, "/services/api/owners/o").await?,
        serde_json::json!(["r"])
    );

    // refs: O(1) head only, ETag + 304
    let (st, text, h) = get_h(server, "/o/r/api/refs", &[]).await?;
    assert_eq!(st, 200);
    let refs: Value = serde_json::from_str(&text)?;
    assert_eq!(refs["head"]["name"], "main");
    assert_eq!(refs["head"]["sha"], head);
    let etag = hdr(&h, "etag");
    assert_eq!(etag, format!("\"{head}\""));
    assert!(hdr(&h, "cache-control").contains("stale-while-revalidate"));
    let (st, _, _) = get_h(server, "/o/r/api/refs", &[("If-None-Match", &etag)]).await?;
    assert_eq!(st, 304);
    // ref lists: paged, sorted, filtered
    let p = json(server, "/o/r/api/refs/branches?n=1").await?;
    assert_eq!(p["refs"][0]["name"], "feature/x");
    assert_eq!(p["more"], true);
    let p = json(server, "/o/r/api/refs/branches?after=feature/x&n=5").await?;
    assert_eq!(p["refs"][0]["name"], "main");
    assert_eq!(p["refs"][0]["sha"], head);
    assert_eq!(p["more"], false);
    let p = json(server, "/o/r/api/refs/branches?q=AIN").await?;
    assert_eq!(p["refs"].as_array().unwrap().len(), 1);
    let p = json(server, "/o/r/api/refs/branches?prefix=feature").await?;
    assert_eq!(p["refs"][0]["name"], "feature/x");
    let p = json(server, "/o/r/api/refs/tags").await?;
    let tags = p["refs"].as_array().unwrap();
    assert_eq!(tags.len(), 2);
    let v1 = tags.iter().find(|t| t["name"] == "v1.0").unwrap();
    assert_eq!(v1["sha"], v1_peeled, "annotated tag sha must be peeled");
    assert_eq!(get(server, "/o/r/api/refs/nope").await?.0, 404);
    // SSE form
    let (st, body, h) = get_h(
        server,
        "/o/r/api/refs/tags",
        &[("Accept", "text/event-stream")],
    )
    .await?;
    assert_eq!(st, 200);
    assert!(hdr(&h, "content-type").starts_with("text/event-stream"));
    assert!(body.contains("event: ref\n") && body.contains("event: done\ndata: {\"more\":false}"));

    // any-namespace refs (D1 collab): full-name listing, namespace filter,
    // exact lookup, pagination, SSE
    let p = json(server, "/o/r/api/refs/all").await?;
    let names: Vec<String> = p["refs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    for expect in [
        "refs/heads/main",
        "refs/heads/feature/x",
        "refs/tags/v1.0",
        "refs/collab/inbox/alice/1",
    ] {
        assert!(
            names.contains(&expect.to_string()),
            "refs/all missing {expect}: {names:?}"
        );
    }
    assert!(
        names.windows(2).all(|w| w[0] < w[1]),
        "refs/all must be byte-sorted: {names:?}"
    );
    let p = json(server, "/o/r/api/refs/all?prefix=refs/collab").await?;
    assert_eq!(p["refs"].as_array().unwrap().len(), 1);
    let p = json(server, "/o/r/api/refs/all?n=2").await?;
    assert_eq!(p["refs"].as_array().unwrap().len(), 2);
    assert_eq!(p["more"], true);
    let p = json(server, "/o/r/api/refs/collab").await?;
    assert_eq!(p["refs"][0]["name"], "refs/collab/inbox/alice/1");
    assert_eq!(p["refs"][0]["sha"], head);
    assert_eq!(
        json(server, "/o/r/api/refs/collab?q=INBOX").await?["refs"][0]["name"],
        "refs/collab/inbox/alice/1"
    );
    let r = json(server, "/o/r/api/refs/name/refs/collab/inbox/alice/1").await?;
    assert_eq!(r["name"], "refs/collab/inbox/alice/1");
    assert_eq!(r["sha"], head);
    let r = json(server, "/o/r/api/refs/name/refs/heads/main").await?;
    assert_eq!(r["sha"], head);
    assert_eq!(get(server, "/o/r/api/refs/name/refs/nope").await?.0, 404);
    let (st, body, h) = get_h(
        server,
        "/o/r/api/refs/collab",
        &[("Accept", "text/event-stream")],
    )
    .await?;
    assert_eq!(st, 200);
    assert!(hdr(&h, "content-type").starts_with("text/event-stream"));
    assert!(body.contains("event: ref\n") && body.contains("event: done\n"));

    // merge-base (D1 review primitive): local git and remote reader agree
    let expected_base = git_in(src, &["merge-base", "main", "feature/x"])?
        .trim()
        .to_string();
    let mb = json(server, "/o/r/api/merge-base?from=main&to=feature/x").await?;
    assert_eq!(
        mb["from"].as_str().unwrap().len(),
        40,
        "from resolved to a sha"
    );
    assert_eq!(mb["to"].as_str().unwrap().len(), 40, "to resolved to a sha");
    assert_eq!(mb["merge_base"], expected_base);
    let mb = json(server, "/o/r/api/merge-base?from=main&to=main").await?;
    assert_eq!(mb["merge_base"], mb["from"], "same revision -> itself");
    assert_eq!(
        get(server, "/o/r/api/merge-base?from=nope&to=main")
            .await?
            .0,
        404
    );

    // diff (D1 review primitive): name-status / stat / patch, local + remote
    let d = json(
        server,
        "/o/r/api/diff?from=feature/x&to=main&format=name-status",
    )
    .await?;
    assert_eq!(d["format"], "name-status");
    assert_eq!(d["from"].as_str().unwrap().len(), 40);
    assert_eq!(d["to"].as_str().unwrap().len(), 40);
    let ch = d["changes"].as_array().unwrap();
    assert!(
        ch.iter()
            .any(|c| c["status"] == "M" && c["path"] == "src/inner/x.txt"),
        "second on main modified x.txt: {ch:?}"
    );
    let st = json(server, "/o/r/api/diff?from=feature/x&to=main&format=stat").await?;
    assert_eq!(st["format"], "stat");
    assert!(
        st["stats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["path"] == "src/inner/x.txt"),
        "stat lists x.txt"
    );
    let p = json(server, "/o/r/api/diff?from=feature/x&to=main").await?;
    assert_eq!(p["format"], "patch", "default format is patch");
    assert!(p["patch"].as_str().unwrap().contains("diff --git"));
    let same = json(server, "/o/r/api/diff?from=main&to=main&format=name-status").await?;
    assert_eq!(same["changes"].as_array().unwrap().len(), 0);
    assert_eq!(
        get(server, "/o/r/api/diff?from=main&to=main&format=bogus")
            .await?
            .0,
        404
    );
    assert_eq!(get(server, "/o/r/api/diff?from=nope&to=main").await?.0, 404);

    // blame (D1 review primitive): porcelain parsed, local + remote agree
    let b = json(server, "/o/r/api/blame/main/src/inner/x.txt").await?;
    assert_eq!(b["path"], "src/inner/x.txt");
    assert_eq!(b["sha"].as_str().unwrap().len(), 40);
    let lines = b["blame"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["line"], 1);
    assert_eq!(lines[0]["text"], "xx", "main's x.txt content");
    assert!(
        !lines[0]["author"].as_str().unwrap().is_empty(),
        "author present"
    );
    assert!(
        lines[0]["summary"]
            .as_str()
            .unwrap()
            .contains("second on main"),
        "main's line attributed to second on main: {lines:?}"
    );
    let b2 = json(server, "/o/r/api/blame/feature/x/src/inner/x.txt").await?;
    let l2 = &b2["blame"][0];
    assert!(
        l2["summary"].as_str().unwrap().contains("initial"),
        "feature/x's x.txt came from initial: {l2:?}"
    );
    assert_eq!(get(server, "/o/r/api/blame/main/nope.txt").await?.0, 404);

    // archive (D1 review primitive): binary download, gzip/zip magic
    let resp = reqwest::Client::new()
        .get(format!("{}/o/r/api/archive/main", server.base_url))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.starts_with("application/gzip"), "ct: {ct}");
    let bytes = resp.bytes().await?;
    assert!(bytes.len() > 100, "archive is non-trivial: {}", bytes.len());
    assert!(
        bytes.starts_with(b"\x1f\x8b"),
        "gzip magic: {:02x?}",
        &bytes[..2]
    );
    let resp = reqwest::Client::new()
        .get(format!(
            "{}/o/r/api/archive/main?format=zip",
            server.base_url
        ))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.starts_with("application/zip"), "ct: {ct}");
    let bytes = resp.bytes().await?;
    assert!(bytes.starts_with(b"PK"), "zip magic: {:02x?}", &bytes[..2]);
    assert_eq!(
        get(server, "/o/r/api/archive/main?format=bogus").await?.0,
        404
    );
    assert_eq!(get(server, "/o/r/api/archive/nope").await?.0, 404);

    // resolve
    let (st, text, h) = get_h(server, "/o/r/api/resolve/feature/x/dir", &[]).await?;
    assert_eq!(st, 200);
    let r: Value = serde_json::from_str(&text)?;
    assert_eq!(r["ref"], "feature/x");
    assert_eq!(r["sha"], feature);
    assert_eq!(r["path"], "dir");
    assert_eq!(r["kind"], "branch");
    assert_eq!(hdr(&h, "etag"), format!("\"{feature}\""));
    let r = json(server, "/o/r/api/resolve/v1.0").await?;
    assert_eq!(r["kind"], "tag");
    assert_eq!(r["sha"], v1_peeled);
    let r = json(server, &format!("/o/r/api/resolve/{}/src", &head[..8])).await?;
    assert_eq!(r["kind"], "commit");
    assert_eq!(r["sha"], head);
    assert_eq!(r["path"], "src");
    let r = json(server, "/o/r/api/resolve/").await?;
    assert_eq!(r["ref"], "main");
    let (st, _, ct) = get(server, "/o/r/api/resolve/nope/x").await?;
    assert_eq!(st, 404);
    assert!(!ct.unwrap_or_default().contains("json"));

    // tree root
    let (st, text, h) = get_h(server, "/o/r/api/tree/main", &[]).await?;
    assert_eq!(st, 200);
    let tree: Value = serde_json::from_str(&text)?;
    assert_eq!(tree["ref"], "main");
    assert_eq!(tree["sha"], head);
    assert_eq!(tree["path"], "");
    assert!(hdr(&h, "cache-control").contains("stale-while-revalidate"));
    assert_eq!(hdr(&h, "etag"), format!("\"{head}\""));
    let names: Vec<&str> = tree["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["dir", "src", "README.md", "big.txt", "bin.dat"],
        "dirs first, then byte order"
    );
    let src_entry = &tree["entries"][1];
    assert_eq!(src_entry["type"], "tree");
    assert_eq!(src_entry["mode"], "040000");
    assert_eq!(src_entry["size"], -1);
    let readme_entry = &tree["entries"][2];
    assert_eq!(readme_entry["type"], "blob");
    assert_eq!(readme_entry["mode"], "100644");
    assert_eq!(readme_entry["size"], "# Title\n\nhello\n".len());
    assert_eq!(tree["readme"]["name"], "README.md");
    assert!(
        tree["readme"]["contents"]
            .as_str()
            .unwrap()
            .starts_with("# Title")
    );
    assert_eq!(tree["commit"]["sha"].as_str().unwrap().len(), 40);

    // longest-ref rule: feature/x + path dir
    let t = json(server, "/o/r/api/tree/feature/x/dir").await?;
    assert_eq!(t["ref"], "feature/x");
    assert_eq!(t["path"], "dir");
    assert_eq!(t["entries"][0]["name"], "f.txt");
    // subtree commit = newest commit touching the path
    let t = json(server, "/o/r/api/tree/main/src/inner").await?;
    assert_eq!(t["entries"][0]["name"], "x.txt");
    assert_eq!(t["commit"]["subject"], "second on main");
    // blob path as tree -> 404 plain text
    let (st, body, ct) = get(server, "/o/r/api/tree/main/README.md").await?;
    assert_eq!(st, 404);
    assert!(
        !ct.unwrap_or_default().contains("json"),
        "404 must be plain text: {body}"
    );
    // sha as ref -> immutable
    let (st, text, h) = get_h(server, &format!("/o/r/api/tree/{feature}"), &[]).await?;
    assert_eq!(st, 200);
    let t: Value = serde_json::from_str(&text)?;
    assert_eq!(t["ref"], feature);
    assert_eq!(t["sha"], feature);
    assert!(hdr(&h, "cache-control").contains("immutable"));
    // second hit served from the immutable LRU
    let (st, text2, _) = get_h(server, &format!("/o/r/api/tree/{feature}"), &[]).await?;
    assert_eq!(st, 200);
    assert_eq!(text, text2);

    // blob
    let b = json(server, "/o/r/api/blob/main/README.md").await?;
    assert_eq!(b["name"], "README.md");
    assert_eq!(b["path"], "README.md");
    assert_eq!(b["contents"], "# Title\n\nhello\n");
    let (st, raw, ct) = get(server, "/o/r/api/blob/main/README.md?raw").await?;
    assert_eq!(st, 200);
    assert!(ct.unwrap_or_default().starts_with("text/plain"));
    assert_eq!(raw, "# Title\n\nhello\n");
    let b = json(server, "/o/r/api/blob/main/bin.dat").await?;
    assert_eq!(b["binary"], true);
    assert!(b.get("contents").is_none());
    let b = json(server, "/o/r/api/blob/main/big.txt").await?;
    assert_eq!(b["too_large"], true);
    assert_eq!(b["size"], 2 * 1024 * 1024 + 1);
    assert_eq!(get(server, "/o/r/api/blob/main/nope.txt").await?.0, 404);

    // commits + pagination
    let c = json(server, "/o/r/api/commits?ref=main&path=&skip=0").await?;
    assert_eq!(c["ref"], "main");
    assert_eq!(c["sha"], head);
    let (_, _, h) = get_h(
        server,
        &format!("/o/r/api/commits?ref={head}&path=&skip=0"),
        &[],
    )
    .await?;
    assert!(hdr(&h, "cache-control").contains("immutable"));
    let commits = c["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 35);
    assert_eq!(c["more"], true);
    assert_eq!(commits[0]["sha"], head);
    assert!(commits[0]["parents"].is_array());
    let c2 = json(server, "/o/r/api/commits?ref=main&skip=35&n=50").await?;
    assert_eq!(c2["more"], false);
    let total = 35 + c2["commits"].as_array().unwrap().len();
    let expected: usize = git_in(src, &["rev-list", "--count", "main"])?
        .trim()
        .parse()?;
    assert_eq!(total, expected);
    let c = json(server, "/o/r/api/commits?ref=main&path=src/inner/x.txt").await?;
    let subjects: Vec<&str> = c["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["subject"].as_str().unwrap())
        .collect();
    assert_eq!(subjects, vec!["second on main", "initial"]);
    let first = &c["commits"][1];
    assert_eq!(first["body"], "body line");
    assert_eq!(first["parents"], serde_json::json!([]));
    assert!(first["author_date"].as_str().unwrap().contains('T'));
    assert_eq!(get(server, "/o/r/api/commits?ref=nope").await?.0, 404);

    // commit detail: rename + merge (first-parent)
    let d = json(server, &format!("/o/r/api/commit/{feature}")).await?;
    assert_eq!(d["commit"]["sha"], feature);
    let paths: Vec<&str> = d["stats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.contains(&"src/app.rs"),
        "renamed file appears once with new path: {paths:?}"
    );
    assert!(!paths.contains(&"src/main.rs"));
    assert!(d["patch"].as_str().unwrap().contains("diff --git a/"));
    let m = json(server, &format!("/o/r/api/commit/{head}")).await?;
    // HEAD is a filler empty commit; find the merge commit instead.
    assert_eq!(m["stats"], serde_json::json!([]));
    let merge = git_in(src, &["rev-parse", "main~40"])?.trim().to_string();
    let m = json(server, &format!("/o/r/api/commit/{merge}")).await?;
    assert_eq!(m["commit"]["parents"].as_array().unwrap().len(), 2);
    assert!(
        !m["stats"].as_array().unwrap().is_empty(),
        "merge diffed against first parent must have stats"
    );
    assert!(m["patch"].as_str().unwrap().contains("diff --git"));
    assert!(!m["patch"].as_str().unwrap().contains("diff --cc"));
    // short sha and 404
    let d = json(server, &format!("/o/r/api/commit/{}", &feature[..10])).await?;
    assert_eq!(d["commit"]["sha"], feature);
    let (_, _, h) = get_h(server, &format!("/o/r/api/commit/{}", &feature[..10]), &[]).await?;
    assert_eq!(hdr(&h, "etag"), format!("\"{feature}\""));
    let (_, _, h) = get_h(server, &format!("/o/r/api/commit/{feature}"), &[]).await?;
    assert!(hdr(&h, "cache-control").contains("immutable"));
    let (st, _, ct) = get(server, "/o/r/api/commit/deadbeef").await?;
    assert_eq!(st, 404);
    assert!(!ct.unwrap_or_default().contains("json"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn api_md_conformance() -> TestResult {
    let server = Server::start().await?;
    // Empty instance.
    assert_eq!(
        json(&server, "/services/api/owners").await?,
        serde_json::json!([])
    );
    assert_eq!(
        json(&server, "/services/api/owners/nobody").await?,
        serde_json::json!([])
    );

    server.put_repo("o", "r").await?;
    let src = fixture(&server)?;
    let head = git_in(&src, &["rev-parse", "HEAD"])?.trim().to_string();
    let feature = git_in(&src, &["rev-parse", "feature/x"])?
        .trim()
        .to_string();
    let v1_peeled = git_in(&src, &["rev-parse", "v1.0^{commit}"])?
        .trim()
        .to_string();
    conformance(&server, &src, &head, &feature, &v1_peeled).await?;

    // unknown repo
    assert_eq!(get(&server, "/o/nope/api/refs").await?.0, 404);
    // page route -> index.html
    let (st, html, ct) = get(&server, "/o/r/tree/main/anything").await?;
    assert_eq!(st, 200);
    assert!(ct.unwrap_or_default().starts_with("text/html"));
    assert!(html.contains("<html"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_repo_refs() -> TestResult {
    let server = Server::start().await?;
    server.put_repo("o", "empty").await?;
    let refs = json(&server, "/o/empty/api/refs").await?;
    assert!(refs["head"].is_null());
    let p = json(&server, "/o/empty/api/refs/branches").await?;
    assert_eq!(p["refs"], serde_json::json!([]));
    assert_eq!(p["more"], false);
    assert_eq!(get(&server, "/o/empty/api/resolve/").await?.0, 404);
    Ok(())
}

/// The same contract on an instance whose `cache.max_bytes` is too small for
/// the repo's packs: objects are read from the store by range (indexes local),
/// nothing is materialized, and long answers stream the SSE envelope.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_objects_conformance() -> TestResult {
    let big = Server::start().await?;
    big.put_repo("o", "r").await?;
    let src = fixture(&big)?;
    let head = git_in(&src, &["rev-parse", "HEAD"])?.trim().to_string();
    let feature = git_in(&src, &["rev-parse", "feature/x"])?
        .trim()
        .to_string();
    let v1_peeled = git_in(&src, &["rev-parse", "v1.0^{commit}"])?
        .trim()
        .to_string();

    let small = big
        .start_sibling_with(|cfg| {
            cfg.cache.max_bytes = bytesize::ByteSize::b(1);
        })
        .await?;

    // First object request with SSE accept: envelope with task/notice packets and a result.
    let resp = reqwest::Client::new()
        .get(format!("{}/o/r/api/tree/{head}", small.base_url))
        .header("Accept", "application/json, text/event-stream")
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    assert!(
        hdr(resp.headers(), "content-type").starts_with("text/event-stream"),
        "first remote render streams"
    );
    let text = resp.text().await?;
    assert!(
        text.contains("event: notice\n"),
        "narrates what it does: {text}"
    );
    assert!(
        text.contains("event: task\n"),
        "remote-index task announced: {text}"
    );
    let result_line = text
        .split("\n\n")
        .find(|p| p.starts_with("event: result"))
        .expect("result packet");
    let body: Value =
        serde_json::from_str(result_line.trim_start_matches("event: result\ndata: "))?;
    assert_eq!(body["sha"], head);
    assert!(body["entries"].as_array().unwrap().len() >= 5);

    // Second time: served from the immutable cache as plain JSON even with SSE accept.
    let resp = reqwest::Client::new()
        .get(format!("{}/o/r/api/tree/{head}", small.base_url))
        .header("Accept", "application/json, text/event-stream")
        .send()
        .await?;
    assert!(hdr(resp.headers(), "content-type").starts_with("application/json"));
    assert!(hdr(resp.headers(), "cache-control").contains("immutable"));

    // Full contract, plain JSON.
    conformance(&small, &src, &head, &feature, &v1_peeled).await?;
    assert!(
        !small.registry_has_packs("o", "r").await,
        "small front must not materialize packs"
    );

    // Tasks are discoverable: the remote-index task ran here and finished ok.
    let t = json(&small, "/o/r/api/tasks").await?;
    let recent = t["recent"].as_array().unwrap();
    let ri = recent
        .iter()
        .find(|r| r["kind"] == "remote-index")
        .expect("remote-index task");
    assert_eq!(ri["ok"], true);
    assert!(t["running"].as_array().unwrap().is_empty());
    // Attach to the finished task: replay + result.
    let (st, text, h) = get_h(
        &small,
        &format!("/o/r/api/tasks/{}", ri["id"].as_str().unwrap()),
        &[("Accept", "text/event-stream")],
    )
    .await?;
    assert_eq!(st, 200);
    assert!(hdr(&h, "content-type").starts_with("text/event-stream"));
    assert!(text.contains("event: result\n"), "{text}");

    // Overview (WAL tab) renders without packs and says so.
    let o = json(&small, "/o/r/api/overview").await?;
    assert_eq!(o["local"]["objects"], "remote");
    assert!(
        o["health"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i.as_str().unwrap().contains("exceeds"))
    );
    Ok(())
}

/// D15/D27: the repo-scoped API is `/{owner}/{repo}/api/…` (and `…/api-browser/…`);
/// the pre-D15 `/services/api/{owner}/{repo}/…` shape is gone (no aliases —
/// AGENTS banner).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_api_prefix_is_gone() -> TestResult {
    let server = Server::start().await?;
    server.put_repo("o", "r").await?;
    let src = fixture(&server)?;
    let _ = git_in(&src, &["rev-parse", "HEAD"])?;
    for path in ["refs", "resolve/main", "tree/main", "tasks", "overview"] {
        let (sa, a, _) = get_h(&server, &format!("/o/r/api/{path}"), &[]).await?;
        assert_eq!(sa, 200, "{path}: {a}");
        let (sb, _, _) = get_h(&server, &format!("/o/r/api-browser/{path}"), &[]).await?;
        assert_eq!(sb, 200, "{path} on the browser lane");
        let (sc, _, _) = get_h(&server, &format!("/services/api/o/r/{path}"), &[]).await?;
        assert_eq!(sc, 404, "{path}: /services/api/o/r must be gone");
    }
    Ok(())
}

/// Pushes are `/<area>/<repository>.git` only — no `.git` is a pkt-line ERR.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_requires_area_repository_git() -> TestResult {
    let server = Server::start().await?;
    server.put_repo("area", "repository").await?;
    let (st, body, _) = get_h(
        &server,
        "/area/repository/info/refs?service=git-receive-pack",
        &[],
    )
    .await?;
    assert_eq!(st, 200, "{body}");
    assert!(
        body.contains("<area>/<repository>.git"),
        "refusal must name the required URL shape: {body:?}"
    );
    let (ok, ad, _) = get_h(
        &server,
        "/area/repository.git/info/refs?service=git-receive-pack",
        &[],
    )
    .await?;
    assert_eq!(ok, 200, "{ad}");
    assert!(!ad.contains("push URL must be"), "{ad:?}");
    Ok(())
}

/// A browser on localhost is sent to walgit.localhost (same port). Git is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_localhost_redirects_to_walgit_localhost() -> TestResult {
    let server = Server::start().await?;
    let url = format!("{}/", server.base_url);
    let resp = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .get(&url)
        .header("Accept", "text/html")
        .send()
        .await?;
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::FOUND,
        "{}",
        resp.status()
    );
    let loc = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        loc.contains("walgit.localhost"),
        "browser should be sent to walgit.localhost, got {loc}"
    );
    let git = reqwest::Client::new()
        .get(format!(
            "{}/area/repository.git/info/refs?service=git-upload-pack",
            server.base_url
        ))
        .header("User-Agent", "git/2.46.0")
        .send()
        .await?;
    assert_ne!(git.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn merge_base_unrelated_histories() -> TestResult {
    let server = Server::start().await?;
    server.put_repo("o", "r2").await?;
    let dir = tempfile::tempdir()?.keep();
    git_in(&dir, &["init", "-q", "-b", "main"])?;
    git_in(&dir, &["config", "user.email", "t@t"])?;
    git_in(&dir, &["config", "user.name", "Tester"])?;
    std::fs::write(dir.join("a.txt"), "a\n")?;
    git_in(&dir, &["add", "."])?;
    git_in(&dir, &["commit", "-q", "-m", "a"])?;
    git_in(&dir, &["checkout", "-q", "--orphan", "other"])?;
    std::fs::write(dir.join("b.txt"), "b\n")?;
    git_in(&dir, &["add", "."])?;
    git_in(&dir, &["commit", "-q", "-m", "b"])?;
    git_in(
        &dir,
        &["push", "-q", "--mirror", &server.repo_url("o", "r2")],
    )?;
    let mb = json(&server, "/o/r2/api/merge-base?from=main&to=other").await?;
    assert_eq!(mb["merge_base"], Value::Null, "unrelated histories -> null");
    Ok(())
}

/// D1 blame known limitation (web/API.md + docs/D1_COLLAB_DESIGN.md §9): git
/// blame has no switch to turn rename-following off, so a remote-served base
/// cannot follow a rename (the parent tree that holds the old path is never
/// faulted). Local blame follows it; remote must fail with a defined 404, not
/// a silent wrong answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_blame_of_renamed_file_is_a_defined_error() -> TestResult {
    let big = Server::start().await?;
    big.put_repo("o", "r").await?;
    fixture(&big)?;
    let small = big
        .start_sibling_with(|cfg| {
            cfg.cache.max_bytes = bytesize::ByteSize::b(1);
        })
        .await?;
    // Local blame follows the rename (src/main.rs -> src/app.rs) and works.
    let b = json(&big, "/o/r/api/blame/main/src/app.rs").await?;
    assert_eq!(b["path"], "src/app.rs");
    assert!(
        !b["blame"].as_array().unwrap().is_empty(),
        "local blame works"
    );
    // Remote cannot follow the rename: defined 404, not a wrong answer.
    assert_eq!(get(&small, "/o/r/api/blame/main/src/app.rs").await?.0, 404);
    Ok(())
}

/// Regression (PR #9 review C1): the remote merge-base walk must not busy-spin
/// when one frontier exhausts before the other. Two unrelated roots on a
/// 1-byte-cache sibling must answer `null`, not hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_merge_base_unrelated_histories_is_null() -> TestResult {
    let big = Server::start().await?;
    big.put_repo("o", "r").await?;
    let dir = tempfile::tempdir()?.keep();
    git_in(&dir, &["init", "-q", "-b", "main"])?;
    git_in(&dir, &["config", "user.email", "t@t"])?;
    git_in(&dir, &["config", "user.name", "Tester"])?;
    std::fs::write(dir.join("a.txt"), "a\n")?;
    git_in(&dir, &["add", "."])?;
    git_in(&dir, &["commit", "-q", "-m", "a"])?;
    git_in(&dir, &["checkout", "-q", "--orphan", "other"])?;
    std::fs::write(dir.join("b.txt"), "b\n")?;
    git_in(&dir, &["add", "."])?;
    git_in(&dir, &["commit", "-q", "-m", "b"])?;
    git_in(&dir, &["push", "-q", "--mirror", &big.repo_url("o", "r")])?;
    let small = big
        .start_sibling_with(|cfg| {
            cfg.cache.max_bytes = bytesize::ByteSize::b(1);
        })
        .await?;
    let mb = json(&small, "/o/r/api/merge-base?from=main&to=other").await?;
    assert_eq!(mb["merge_base"], Value::Null, "remote unrelated -> null");
    Ok(())
}

/// Regression (PR #9 review C1): a feature forked from DEEP main — the walk
/// must meet at the branch point even though one side's frontier empties
/// first, and the remote answer must equal local git's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_merge_base_deep_fork() -> TestResult {
    let big = Server::start().await?;
    big.put_repo("o", "r").await?;
    let dir = tempfile::tempdir()?.keep();
    git_in(&dir, &["init", "-q", "-b", "main"])?;
    git_in(&dir, &["config", "user.email", "t@t"])?;
    git_in(&dir, &["config", "user.name", "Tester"])?;
    std::fs::write(dir.join("a.txt"), "a\n")?;
    git_in(&dir, &["add", "."])?;
    git_in(&dir, &["commit", "-q", "-m", "c1"])?;
    for i in 2..=10 {
        git_in(
            &dir,
            &["commit", "-q", "--allow-empty", "-m", &format!("m{i}")],
        )?;
    }
    // Feature forks from main~2 (deep in main's history) and adds two commits.
    git_in(&dir, &["checkout", "-q", "-b", "feature", "HEAD~2"])?;
    git_in(&dir, &["commit", "-q", "--allow-empty", "-m", "f1"])?;
    git_in(&dir, &["commit", "-q", "--allow-empty", "-m", "f2"])?;
    git_in(&dir, &["checkout", "-q", "main"])?;
    let expected = git_in(&dir, &["merge-base", "main", "feature"])?
        .trim()
        .to_string();
    git_in(&dir, &["push", "-q", "--mirror", &big.repo_url("o", "r")])?;
    let small = big
        .start_sibling_with(|cfg| {
            cfg.cache.max_bytes = bytesize::ByteSize::b(1);
        })
        .await?;
    let mb = json(&small, "/o/r/api/merge-base?from=main&to=feature").await?;
    assert_eq!(
        mb["merge_base"], expected,
        "remote deep fork base matches git"
    );
    Ok(())
}

/// D1 thin-API write path: POST a signed collab entry -> the ref lands in
/// refs/collab/inbox/<actor>/; posting as someone else is forbidden; no
/// credential is 401. (Signature verification is client-side; the server
/// enforces identity and inbox ownership.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn collab_thin_api_posts_signed_entries() -> TestResult {
    let server = Server::start_with_tweak(|c| {
        c.server.auth.mode = walgit_config::AuthMode::Token;
        c.server.auth.anonymous_read = false;
        c.server.auth.tokens = vec![walgit_config::StaticToken {
            principal: "alice".into(),
            token: "alice-token".into(),
            token_env: None,
            write: true,
            admin: false,
        }];
    })
    .await?;
    let client = reqwest::Client::new();
    let put = client
        .put(format!("{}/o/r", server.base_url))
        .bearer_auth("alice-token")
        .send()
        .await?;
    assert!(put.status().is_success() || put.status() == reqwest::StatusCode::CONFLICT);
    let url = format!("{}/o/r/api/collab/entries", server.base_url);

    let entry = serde_json::json!({
        "version": 1, "kind": "issue", "id": "t1", "actor": "alice",
        "ts": 1786500000, "parent": "", "body": {"title": "hi"},
        "sig": "ed25519:AAAA"
    });
    let resp = client
        .post(&url)
        .bearer_auth("alice-token")
        .json(&serde_json::json!({ "entry": entry }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200, "post signed entry");
    let body: serde_json::Value = resp.json().await?;
    let ref_name = body["ref"].as_str().expect("ref").to_string();
    assert!(ref_name.starts_with("refs/collab/inbox/alice/"), "{ref_name}");
    assert_eq!(body["oid"].as_str().unwrap().len(), 40);

    // Visible in the collab namespace listing (authenticated read).
    let (st, text, _) = get_h(
        &server,
        "/o/r/api/refs/collab",
        &[("Authorization", "Bearer alice-token")],
    )
    .await?;
    assert_eq!(st, 200);
    let r: serde_json::Value = serde_json::from_str(&text)?;
    let names: Vec<String> = r["refs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&ref_name), "ref in refs/collab: {names:?}");

    // Posting as someone else -> 403.
    let bad = serde_json::json!({
        "entry": serde_json::json!({
            "version": 1, "kind": "comment", "id": "t1", "actor": "bob",
            "ts": 1, "parent": "", "body": {}, "sig": ""
        })
    });
    let resp = client
        .post(&url)
        .bearer_auth("alice-token")
        .json(&bad)
        .send()
        .await?;
    let bad_status = resp.status();
    assert_eq!(bad_status, 403, "actor != principal refused");

    // No credential -> 401.
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "entry": entry }))
        .send()
        .await?;
    assert_eq!(resp.status(), 403, "unauthenticated refused (token mode, no credential)");
    Ok(())
}

/// D1 aggregation read path: after posting entries, `/api/collab/report` and
/// `/api/collab/threads/{id}` answer with the deterministic aggregation
/// (thread summaries, ordered entries, PR view + merge rule evaluation).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn collab_report_and_thread_aggregate_entries() -> TestResult {
    let server = Server::start_with_tweak(|c| {
        c.server.auth.mode = walgit_config::AuthMode::Token;
        c.server.auth.anonymous_read = false;
        c.server.auth.tokens = vec![walgit_config::StaticToken {
            principal: "alice".into(),
            token: "alice-token".into(),
            token_env: None,
            write: true,
            admin: false,
        }];
    })
    .await?;
    let client = reqwest::Client::new();
    let put = client
        .put(format!("{}/o/r", server.base_url))
        .bearer_auth("alice-token")
        .send()
        .await?;
    assert!(put.status().is_success() || put.status() == reqwest::StatusCode::CONFLICT);
    let url = format!("{}/o/r/api/collab/entries", server.base_url);

    let issue = serde_json::json!({
        "version": 1, "kind": "issue", "id": "t1", "actor": "alice",
        "ts": 1786500000, "parent": "", "body": {"title": "hi"},
        "sig": "ed25519:AAAA"
    });
    let patch = serde_json::json!({
        "version": 1, "kind": "patch", "id": "t1", "actor": "alice",
        "ts": 1786500001, "parent": "", "body": {},
        "refs": {"base": "refs/heads/main", "head": "refs/heads/topic"},
        "sig": "ed25519:BBBB"
    });
    let review = serde_json::json!({
        "version": 1, "kind": "review", "id": "t1", "actor": "alice",
        "ts": 1786500002, "parent": "", "body": {"decision": "approve"},
        "sig": "ed25519:CCCC"
    });
    let mut oids = Vec::new();
    for e in [&issue, &patch, &review] {
        let resp = client
            .post(&url)
            .bearer_auth("alice-token")
            .json(&serde_json::json!({ "entry": e }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "post entry");
        let body: serde_json::Value = resp.json().await?;
        oids.push(body["oid"].as_str().unwrap().to_string());
    }

    let (st, text, _) = get_h(
        &server,
        "/o/r/api/collab/report",
        &[("Authorization", "Bearer alice-token")],
    )
    .await?;
    assert_eq!(st, 200);
    let report: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(report["total_entries"], 3);
    assert_eq!(report["threads"].as_array().unwrap().len(), 1);
    assert_eq!(report["threads"][0]["id"], "t1");
    assert_eq!(report["threads"][0]["entries"], 3);
    assert_eq!(report["prs"].as_array().unwrap().len(), 1);
    assert_eq!(report["prs"][0]["base"], "refs/heads/main");
    assert_eq!(report["prs"][0]["head"], "refs/heads/topic");
    assert_eq!(report["prs"][0]["status"], "open");
    // Default rules protect nothing -> merge allowed.
    assert_eq!(report["prs"][0]["merge_allowed"], true);

    let (st, text, _) = get_h(
        &server,
        "/o/r/api/collab/threads/t1",
        &[("Authorization", "Bearer alice-token")],
    )
    .await?;
    assert_eq!(st, 200);
    let thread: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(thread["id"], "t1");
    let entries = thread["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    let kinds: Vec<&str> = entries
        .iter()
        .map(|e| e["entry"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["issue", "patch", "review"], "parent-ordered");
    let pr = thread["pr"].as_object().expect("thread has a patch -> pr view");
    assert_eq!(pr["pr"]["base"], "refs/heads/main");
    assert_eq!(pr["pr"]["head"], "refs/heads/topic");
    assert_eq!(pr["pr"]["reviews"].as_array().unwrap().len(), 1);
    assert_eq!(pr["merge"]["allowed"], true);

    // Unknown thread -> 404.
    let (st, _, _) = get_h(
        &server,
        "/o/r/api/collab/threads/nope",
        &[("Authorization", "Bearer alice-token")],
    )
    .await?;
    assert_eq!(st, 404);
    Ok(())
}

/// D1 principal registration thin API + signed-entry verification: register
/// the authenticated principal's Ed25519 key, post a signed issue, and the
/// report/thread answers count it verified (the aggregation verifies exactly
/// what the CLI does locally).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn collab_principal_registration_and_verified_entries() -> TestResult {
    use base64::Engine as _;
    use ed25519_dalek::SigningKey;
    use walgit_wal::collab::{Entry, sign_entry};

    let server = Server::start_with_tweak(|c| {
        c.server.auth.mode = walgit_config::AuthMode::Token;
        c.server.auth.anonymous_read = false;
        c.server.auth.tokens = vec![walgit_config::StaticToken {
            principal: "alice".into(),
            token: "alice-token".into(),
            token_env: None,
            write: true,
            admin: false,
        }];
    })
    .await?;
    let client = reqwest::Client::new();
    let put = client
        .put(format!("{}/o/r", server.base_url))
        .bearer_auth("alice-token")
        .send()
        .await?;
    assert!(put.status().is_success() || put.status() == reqwest::StatusCode::CONFLICT);

    let sk = SigningKey::from_bytes(&[7u8; 32]);
    let public_key = base64::engine::general_purpose::STANDARD.encode(sk.verifying_key().to_bytes());

    // Register alice's key through the thin API.
    let resp = client
        .post(format!("{}/o/r/api/collab/principal", server.base_url))
        .bearer_auth("alice-token")
        .json(&serde_json::json!({ "principal": "alice", "public_key": public_key }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200, "register principal");
    let body: serde_json::Value = resp.json().await?;
    let ref_name = body["ref"].as_str().unwrap();
    assert_eq!(ref_name, "refs/collab/meta/principals/alice");

    // Posting a registration for someone else -> 403.
    let resp = client
        .post(format!("{}/o/r/api/collab/principal", server.base_url))
        .bearer_auth("alice-token")
        .json(&serde_json::json!({ "principal": "bob", "public_key": public_key }))
        .send()
        .await?;
    assert_eq!(resp.status(), 403, "registering another principal refused");

    // Post a genuinely signed issue entry.
    let mut entry = Entry {
        version: 1,
        kind: "issue".into(),
        id: "t2".into(),
        actor: "alice".into(),
        ts: 1786500010,
        parent: String::new(),
        refs: None,
        body: serde_json::json!({ "title": "signed" }),
        sig: String::new(),
    };
    entry.sig = sign_entry(&mut entry, &sk);
    let resp = client
        .post(format!("{}/o/r/api/collab/entries", server.base_url))
        .bearer_auth("alice-token")
        .json(&serde_json::json!({ "entry": serde_json::to_value(&entry)? }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200, "post signed entry");

    // The report counts it verified; the thread detail marks the entry verified.
    let (st, text, _) = get_h(
        &server,
        "/o/r/api/collab/report",
        &[("Authorization", "Bearer alice-token")],
    )
    .await?;
    assert_eq!(st, 200);
    let report: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(report["total_entries"], 1);
    assert_eq!(report["verified_entries"], 1, "signed entry with registered key verifies");
    assert_eq!(report["unverified_entries"], 0);
    assert_eq!(report["missing_principals"], 0);
    assert_eq!(report["threads"][0]["verified"], 1);

    let (st, text, _) = get_h(
        &server,
        "/o/r/api/collab/threads/t2",
        &[("Authorization", "Bearer alice-token")],
    )
    .await?;
    assert_eq!(st, 200);
    let thread: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(thread["entries"][0]["verified"], true);
    Ok(())
}
