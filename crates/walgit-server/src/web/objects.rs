//! Object access for the web API when a repository's packs are **not** on
//! this instance (`ObjectAccess::Remote`): read what a request needs from the
//! WAL pack set by range (`walgit_wal::RemotePacks`), fault those objects into
//! the local loose store, and let the unmodified `git` plumbing the renderers
//! already use run against them. History questions (`git log`) cannot be
//! answered by faulting, so the commit walks live here, over the same reader.
//!
//! Everything reports what it is doing through a [`Reporter`] so the SSE
//! envelope can narrate ("Reading tree areas/core…", "Walked 300 commits…").

use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use gix_hash::ObjectId;
use gix_object::Kind;
use walgit_wal::remote::Obj;
use walgit_wal::{RemotePacks, Reporter};

use crate::error::ApiError;

/// Cap on objects faulted for one commit diff (pathological commits get a
/// clear error instead of a multi-minute range-read storm).
const MAX_DIFF_OBJECTS: usize = 20_000;
/// Budget for "newest commit touching path" (tree header).
const NEWEST_BUDGET: usize = 3_000;
/// Budget for a history page walk (skip + n commits, plus skipped TREESAME ones).
const WALK_BUDGET: usize = 50_000;
/// Budget for a merge-base bidirectional walk (one commit popped per side).
/// Each pop is a serial range read on the remote reader; 50k could stall a
/// request for minutes, so deep/unrelated histories fail fast instead.
const MERGE_BASE_BUDGET: usize = 5_000;
/// Budget for a blame history walk (commits visited before git blame runs).
const BLAME_BUDGET: usize = 3_000;
/// Budget for a whole-tree archive fault (trees + blobs, deduped).
const ARCHIVE_OBJECT_BUDGET: usize = 100_000;

/// Test hook (same pattern as the `WALGIT_TEST_*` knobs in walgit-wal):
/// shrink a budget from the environment so the 503 paths are exercisable in
/// tests without building a 100 k-object fixture.
fn test_budget(env: &str, default: usize) -> usize {
    std::env::var(env)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub struct Remote {
    pub packs: Arc<RemotePacks>,
    pub local: walgit_git::LocalRepo,
    pub reporter: Reporter,
    faulted: parking_lot::Mutex<HashSet<ObjectId>>,
}

fn not_found(m: impl Into<String>) -> ApiError {
    ApiError::NotFound(m.into())
}
fn wal(e: walgit_wal::WalError) -> ApiError {
    ApiError::Internal(format!("remote objects: {e}"))
}

/// A parsed commit (what the walks and renderers need).
#[derive(Clone)]
pub struct CommitMeta {
    pub id: ObjectId,
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: String,
    pub author_email: String,
    pub author_date: String,
    pub committer: String,
    pub commit_date: String,
    pub commit_time: i64,
    pub subject: String,
    pub body: String,
}

pub struct TreeEntryRef {
    pub name: Vec<u8>,
    pub oid: ObjectId,
    pub mode: gix_object::tree::EntryMode,
}

impl Remote {
    pub fn new(packs: Arc<RemotePacks>, local: walgit_git::LocalRepo, reporter: Reporter) -> Self {
        Remote {
            packs,
            local,
            reporter,
            faulted: parking_lot::Mutex::new(HashSet::new()),
        }
    }

    pub fn hash(&self) -> gix_hash::Kind {
        self.packs.hash()
    }

    /// Read an object (404 when absent).
    pub async fn get(&self, oid: &gix_hash::oid) -> Result<Arc<Obj>, ApiError> {
        self.packs
            .find(oid)
            .await
            .map_err(wal)?
            .ok_or_else(|| not_found(format!("object {oid} not in the pack set")))
    }

    /// Read + write into the local loose store (so git can see it).
    pub async fn fault(&self, oid: &gix_hash::oid) -> Result<Arc<Obj>, ApiError> {
        let o = self.get(oid).await?;
        self.write_local(oid, &o)?;
        Ok(o)
    }

    /// Fault a batch concurrently (bounded), skipping what is already local.
    pub async fn fault_many(&self, oids: &[ObjectId]) -> Result<(), ApiError> {
        const PAR: usize = 32;
        let todo: Vec<ObjectId> = {
            let done = self.faulted.lock();
            oids.iter()
                .filter(|o| !done.contains(*o))
                .copied()
                .collect()
        };
        for chunk in todo.chunks(PAR) {
            let results = futures::future::join_all(chunk.iter().map(|o| self.get(o))).await;
            for (oid, r) in chunk.iter().zip(results) {
                let o = r?;
                self.write_local(oid, &o)?;
            }
        }
        Ok(())
    }

    fn write_local(&self, oid: &gix_hash::oid, o: &Obj) -> Result<(), ApiError> {
        if self.faulted.lock().contains(oid) {
            return Ok(());
        }
        self.local
            .write_loose_object(o.kind, oid, &o.data)
            .map_err(|e| ApiError::Internal(format!("fault object {oid}: {e}")))?;
        self.faulted.lock().insert(oid.to_owned());
        Ok(())
    }

    pub async fn kind_and_size(
        &self,
        oid: &gix_hash::oid,
    ) -> Result<Option<(Kind, u64)>, ApiError> {
        self.packs.header(oid).await.map_err(wal)
    }

    /// `rev-parse --verify <rev>^{commit}` without objects on disk: full or
    /// abbreviated sha (unique prefix), tags peeled.
    pub async fn resolve_commitish(&self, rev: &str) -> Result<ObjectId, ApiError> {
        let hex = rev.trim();
        let looks_hex = hex.len() >= 4
            && hex.len() <= self.hash().len_in_hex()
            && hex.bytes().all(|b| b.is_ascii_hexdigit());
        if !looks_hex {
            return Err(not_found(format!("unknown revision {rev}")));
        }
        let oid = if hex.len() == self.hash().len_in_hex() {
            let oid = ObjectId::from_hex(hex.as_bytes())
                .map_err(|_| not_found(format!("unknown revision {rev}")))?;
            if !self.packs.contains(&oid) {
                return Err(not_found(format!("unknown revision {rev}")));
            }
            oid
        } else {
            let prefix = gix_hash::Prefix::from_hex(hex)
                .map_err(|_| not_found(format!("unknown revision {rev}")))?;
            match self.packs.lookup_prefix(prefix) {
                Ok(Some(oid)) => oid,
                Ok(None) => return Err(not_found(format!("unknown revision {rev}"))),
                Err(_) => return Err(not_found(format!("ambiguous revision {rev}"))),
            }
        };
        self.peel_to_commit(oid).await
    }

    async fn peel_to_commit(&self, mut oid: ObjectId) -> Result<ObjectId, ApiError> {
        for _ in 0..16 {
            let (kind, _) = self
                .kind_and_size(&oid)
                .await?
                .ok_or_else(|| not_found(format!("object {oid}")))?;
            match kind {
                Kind::Commit => return Ok(oid),
                Kind::Tag => {
                    let o = self.get(&oid).await?;
                    let tag = gix_object::TagRef::from_bytes(&o.data, self.hash())
                        .map_err(|e| ApiError::Internal(format!("parse tag {oid}: {e}")))?;
                    oid = tag.target();
                }
                _ => return Err(not_found(format!("{oid} is not a commit"))),
            }
        }
        Err(not_found("tag chain too deep"))
    }

    pub async fn commit(&self, oid: &gix_hash::oid) -> Result<CommitMeta, ApiError> {
        let o = self.get(oid).await?;
        if o.kind != Kind::Commit {
            return Err(not_found(format!("{oid} is not a commit")));
        }
        parse_commit(oid.to_owned(), &o.data, self.hash())
    }

    pub async fn tree_entries(&self, oid: &gix_hash::oid) -> Result<Vec<TreeEntryRef>, ApiError> {
        let o = self.get(oid).await?;
        if o.kind != Kind::Tree {
            return Err(not_found(format!("{oid} is not a tree")));
        }
        parse_tree(&o.data, self.hash())
    }

    /// Walk `path` from `tree`; returns the entry (oid, mode) at the end, None if absent.
    pub async fn lookup_path(
        &self,
        tree: ObjectId,
        path: &str,
    ) -> Result<Option<(ObjectId, gix_object::tree::EntryMode)>, ApiError> {
        let mut cur = tree;
        let mut mode: Option<gix_object::tree::EntryMode> = None;
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            let entries = self.tree_entries(&cur).await?;
            let Some(e) = entries.into_iter().find(|e| e.name == seg.as_bytes()) else {
                return Ok(None);
            };
            cur = e.oid;
            mode = Some(e.mode);
            if !e.mode.is_tree() {
                // more segments after a blob => absent
                continue;
            }
        }
        match mode {
            None => Ok(Some((cur, gix_object::tree::EntryKind::Tree.into()))),
            Some(m) => Ok(Some((cur, m))),
        }
    }

    /// Fault the commit, the trees along `path` and the target (tree or blob).
    /// Returns the target entry. 404 if the path does not exist.
    pub async fn fault_path(
        &self,
        commit: &gix_hash::oid,
        path: &str,
    ) -> Result<(CommitMeta, ObjectId, gix_object::tree::EntryMode), ApiError> {
        let c = self.commit(commit).await?;
        self.fault(commit).await?;
        let mut cur = c.tree;
        let mut mode: gix_object::tree::EntryMode = gix_object::tree::EntryKind::Tree.into();
        self.fault(&cur).await?;
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        for (i, seg) in segs.iter().enumerate() {
            let entries = self.tree_entries(&cur).await?;
            let Some(e) = entries.into_iter().find(|e| e.name == seg.as_bytes()) else {
                return Err(not_found(format!(
                    "path '{}' does not exist in {}",
                    path, commit
                )));
            };
            cur = e.oid;
            mode = e.mode;
            if e.mode.is_tree() {
                self.fault(&cur).await?;
            } else if i + 1 < segs.len() {
                return Err(not_found(format!(
                    "path '{}' does not exist in {}",
                    path, commit
                )));
            } else if e.mode.is_blob() {
                // blob: caller decides whether to fault (size check)
            }
        }
        Ok((c, cur, mode))
    }

    /// Oid of `path` in the commit's tree (None if absent). Cached per tree.
    async fn path_oid(
        &self,
        cache: &mut HashMap<ObjectId, Option<ObjectId>>,
        tree: ObjectId,
        path: &str,
    ) -> Result<Option<ObjectId>, ApiError> {
        if let Some(v) = cache.get(&tree) {
            return Ok(*v);
        }
        let v = self.lookup_path(tree, path).await?.map(|(o, _)| o);
        cache.insert(tree, v);
        Ok(v)
    }

    /// History walk in committer-date order (git log's default for non-topo),
    /// with git's default history simplification for `path` (a commit TREESAME
    /// to a parent is dropped and only that parent is followed). Returns up to
    /// `want` shown commits.
    pub async fn walk(
        &self,
        start: ObjectId,
        path: Option<&str>,
        want: usize,
        label: &str,
    ) -> Result<Vec<CommitMeta>, ApiError> {
        self.walk_bounded(
            start,
            path.filter(|p| !p.is_empty()),
            want,
            label,
            WALK_BUDGET,
        )
        .await
    }

    /// `git log -1 <sha> -- <path>` with a budget; None when not found in time.
    pub async fn newest_touching(
        &self,
        start: ObjectId,
        path: &str,
    ) -> Result<Option<CommitMeta>, ApiError> {
        if path.is_empty() {
            return Ok(Some(self.commit(&start).await?));
        }
        // Bounded: reuse walk with a small budget via a local loop.
        let label = format!("Finding the latest commit touching {path}");
        let mut res = self
            .walk_bounded(start, Some(path), 1, &label, NEWEST_BUDGET)
            .await?;
        Ok(res.pop())
    }

    /// A merge base of `a` and `b` (hex shas), by bounded bidirectional walk
    /// over the pack set — no objects on disk needed. Returns the first common
    /// ancestor found: for the branch-from-trunk shape this is the branch
    /// point; criss-cross histories may differ from `git merge-base`'s
    /// preferred one. `None` when the histories are unrelated; 503 when the
    /// walk budget is exhausted (paths too far apart for a remote reader).
    pub async fn merge_base(&self, a: &str, b: &str) -> Result<Option<ObjectId>, ApiError> {
        let a = ObjectId::from_hex(a.as_bytes())
            .map_err(|_| not_found(format!("invalid commit sha {a}")))?;
        let b = ObjectId::from_hex(b.as_bytes())
            .map_err(|_| not_found(format!("invalid commit sha {b}")))?;
        if a == b {
            return Ok(Some(a));
        }
        let mut fa: VecDeque<ObjectId> = VecDeque::from([a]);
        let mut fb: VecDeque<ObjectId> = VecDeque::from([b]);
        let mut sa: HashSet<ObjectId> = HashSet::from([a]);
        let mut sb: HashSet<ObjectId> = HashSet::from([b]);
        let budget = test_budget("WALGIT_TEST_MERGE_BASE_BUDGET", MERGE_BASE_BUDGET);
        let mut popped = 0usize;
        while !fa.is_empty() || !fb.is_empty() {
            // Expand the smaller frontier so the sides meet in the middle; a
            // side whose frontier is exhausted must not be re-picked (otherwise
            // `0 <= n` selects the empty side forever and the loop busy-spins
            // on the async runtime without consuming budget).
            let pick_a = if fa.is_empty() {
                false
            } else if fb.is_empty() {
                true
            } else {
                fa.len() <= fb.len()
            };
            let (front, seen_self, seen_other) = if pick_a {
                (&mut fa, &mut sa, &sb)
            } else {
                (&mut fb, &mut sb, &sa)
            };
            let Some(oid) = front.pop_front() else {
                continue;
            };
            popped += 1;
            if popped > budget {
                self.reporter
                    .notice(format!("merge-base: gave up after {budget} commits"));
                return Err(ApiError::ServiceUnavailable(format!(
                    "merge-base walk exceeded {budget} commits; the histories are too far apart (or unrelated) for the remote reader"
                )));
            }
            if popped.is_multiple_of(100) {
                self.reporter.bar(
                    "Finding merge base".to_string(),
                    popped as u64,
                    None,
                    "commits",
                );
            }
            if seen_other.contains(&oid) {
                return Ok(Some(oid));
            }
            let meta = self.commit(&oid).await?;
            for par in meta.parents {
                if seen_other.contains(&par) {
                    return Ok(Some(par));
                }
                if seen_self.insert(par) {
                    front.push_back(par);
                }
            }
        }
        Ok(None)
    }

    async fn walk_bounded(
        &self,
        start: ObjectId,
        path: Option<&str>,
        want: usize,
        label: &str,
        budget: usize,
    ) -> Result<Vec<CommitMeta>, ApiError> {
        #[derive(PartialEq, Eq)]
        struct Item(i64, u64, ObjectId);
        impl Ord for Item {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                self.0.cmp(&o.0).then_with(|| o.1.cmp(&self.1))
            }
        }
        impl PartialOrd for Item {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        let mut heap = BinaryHeap::new();
        let mut seen: HashSet<ObjectId> = HashSet::new();
        let mut metas: HashMap<ObjectId, CommitMeta> = HashMap::new();
        let mut path_cache: HashMap<ObjectId, Option<ObjectId>> = HashMap::new();
        let mut seq = 0u64;
        let start_meta = self.commit(&start).await?;
        heap.push(Item(start_meta.commit_time, seq, start));
        metas.insert(start, start_meta);
        seen.insert(start);
        let mut out = Vec::new();
        let mut popped = 0usize;
        while let Some(Item(_, _, oid)) = heap.pop() {
            popped += 1;
            if popped > budget {
                self.reporter
                    .notice(format!("{label}: gave up after {budget} commits"));
                break;
            }
            if popped % 100 == 0 {
                self.reporter
                    .bar(label.to_string(), popped as u64, None, "commits");
            }
            let meta = match metas.remove(&oid) {
                Some(m) => m,
                None => self.commit(&oid).await?,
            };
            let mut follow: Vec<ObjectId> = meta.parents.clone();
            let mut show = true;
            if let Some(p) = path {
                let mine = self.path_oid(&mut path_cache, meta.tree, p).await?;
                if meta.parents.is_empty() {
                    show = mine.is_some();
                } else {
                    let mut treesame_parent = None;
                    for par in &meta.parents {
                        let pm = match metas.get(par) {
                            Some(m) => m.clone(),
                            None => {
                                let m = self.commit(par).await?;
                                metas.insert(*par, m.clone());
                                m
                            }
                        };
                        let theirs = self.path_oid(&mut path_cache, pm.tree, p).await?;
                        if theirs == mine {
                            treesame_parent = Some(*par);
                            break;
                        }
                    }
                    if let Some(tp) = treesame_parent {
                        show = false;
                        follow = vec![tp];
                    }
                }
            }
            if show {
                out.push(meta.clone());
                if out.len() >= want {
                    break;
                }
            }
            for par in follow {
                if seen.insert(par) {
                    seq += 1;
                    let pm = match metas.get(&par) {
                        Some(m) => m.clone(),
                        None => {
                            let m = self.commit(&par).await?;
                            metas.insert(par, m.clone());
                            m
                        }
                    };
                    heap.push(Item(pm.commit_time, seq, par));
                }
            }
        }
        Ok(out)
    }

    /// Fault everything `git show -M --first-parent` needs for `commit`: the
    /// commit, its first parent, every tree on a differing path, and the
    /// blobs of changed entries (both sides). Root commits diff against the
    /// empty tree.
    pub async fn fault_commit_diff(&self, commit: &gix_hash::oid) -> Result<CommitMeta, ApiError> {
        let c = self.commit(commit).await?;
        self.fault(commit).await?;
        // The renderer diffs against the first parent only
        // (`--diff-merges=first-parent`); git still parses every parent and
        // peeks at their root trees, so fault those (one object each) but walk
        // the diff for the first parent alone — a merge into a monorepo trunk
        // otherwise pulls the whole other-branch delta (20 k+ objects, 503).
        let mut stack: Vec<(Option<ObjectId>, Option<ObjectId>)> = Vec::new();
        if c.parents.is_empty() {
            stack.push((None, Some(c.tree)));
        }
        for (i, p) in c.parents.iter().enumerate() {
            let pm = self.commit(p).await?;
            self.fault(p).await?;
            if i == 0 {
                stack.push((Some(pm.tree), Some(c.tree)));
            } else {
                self.fault(&pm.tree).await?;
            }
        }
        self.reporter.notice(format!(
            "Reading the trees and blobs changed by {}",
            &c.id.to_hex().to_string()[..12]
        ));
        self.fault_tree_pairs(stack).await?;
        Ok(c)
    }

    /// Fault every tree and blob the diff between `a` and `b` (hex shas)
    /// touches into the local loose store, so an unmodified `git diff` can run
    /// against them. `a == b` faults nothing.
    pub async fn fault_diff(&self, a: &str, b: &str) -> Result<(), ApiError> {
        let a = ObjectId::from_hex(a.as_bytes())
            .map_err(|_| not_found(format!("invalid commit sha {a}")))?;
        let b = ObjectId::from_hex(b.as_bytes())
            .map_err(|_| not_found(format!("invalid commit sha {b}")))?;
        if a == b {
            return Ok(());
        }
        let ca = self.commit(&a).await?;
        self.fault(&a).await?;
        let cb = self.commit(&b).await?;
        self.fault(&b).await?;
        let (sa, sb) = (
            a.to_hex()
                .to_string()
                .get(..12)
                .unwrap_or_default()
                .to_string(),
            b.to_hex()
                .to_string()
                .get(..12)
                .unwrap_or_default()
                .to_string(),
        );
        self.reporter
            .notice(format!("Reading the diff between {sa} and {sb}"));
        self.fault_tree_pairs(vec![(Some(ca.tree), Some(cb.tree))])
            .await
    }

    /// Level-parallel faulting of a set of tree pairs (`None` = empty tree):
    /// every tree of the current level is faulted in one concurrent batch
    /// (range reads ~50 ms each; serially a large repository commit took 300
    /// round trips = 15 s), then merge-walked; the blobs it references are
    /// faulted in a second batch. Rounds ≈ path depth. Bounded by
    /// MAX_DIFF_OBJECTS.
    async fn fault_tree_pairs(
        &self,
        mut stack: Vec<(Option<ObjectId>, Option<ObjectId>)>,
    ) -> Result<(), ApiError> {
        let budget = test_budget("WALGIT_TEST_DIFF_BUDGET", MAX_DIFF_OBJECTS);
        let mut count = 0usize;
        while !stack.is_empty() {
            let level = std::mem::take(&mut stack);
            let mut want: Vec<ObjectId> = Vec::new();
            for (a, b) in &level {
                want.extend(a.iter().copied());
                want.extend(b.iter().copied());
            }
            want.sort_unstable();
            want.dedup();
            count += want.len();
            if count > budget {
                return Err(ApiError::ServiceUnavailable(format!(
                    "diff touches more than {budget} objects; too large to render from the remote pack set"
                )));
            }
            self.fault_many(&want).await?;
            self.reporter
                .bar("Reading changed objects", count as u64, None, "objects");
            let mut blobs: Vec<ObjectId> = Vec::new();
            for (a, b) in level {
                let ea = match a {
                    Some(t) => self.tree_entries(&t).await?,
                    None => Vec::new(),
                };
                let eb = match b {
                    Some(t) => self.tree_entries(&t).await?,
                    None => Vec::new(),
                };
                // Merge-walk by git tree order.
                let (mut i, mut j) = (0, 0);
                while i < ea.len() || j < eb.len() {
                    let ord = match (ea.get(i), eb.get(j)) {
                        (Some(x), Some(y)) => {
                            tree_cmp(&x.name, x.mode.is_tree(), &y.name, y.mode.is_tree())
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => break,
                    };
                    match ord {
                        std::cmp::Ordering::Equal => {
                            let (x, y) = (&ea[i], &eb[j]);
                            i += 1;
                            j += 1;
                            if x.oid == y.oid && x.mode == y.mode {
                                continue;
                            }
                            match (x.mode.is_tree(), y.mode.is_tree()) {
                                (true, true) => stack.push((Some(x.oid), Some(y.oid))),
                                (true, false) => {
                                    stack.push((Some(x.oid), None));
                                    if y.mode.is_blob_or_symlink() {
                                        blobs.push(y.oid);
                                    }
                                }
                                (false, true) => {
                                    stack.push((None, Some(y.oid)));
                                    if x.mode.is_blob_or_symlink() {
                                        blobs.push(x.oid);
                                    }
                                }
                                (false, false) => {
                                    if x.mode.is_blob_or_symlink() {
                                        blobs.push(x.oid);
                                    }
                                    if y.mode.is_blob_or_symlink() && y.oid != x.oid {
                                        blobs.push(y.oid);
                                    }
                                }
                            }
                        }
                        std::cmp::Ordering::Less => {
                            let x = &ea[i];
                            i += 1;
                            if x.mode.is_tree() {
                                stack.push((Some(x.oid), None));
                            } else if x.mode.is_blob_or_symlink() {
                                blobs.push(x.oid);
                            }
                        }
                        std::cmp::Ordering::Greater => {
                            let y = &eb[j];
                            j += 1;
                            if y.mode.is_tree() {
                                stack.push((None, Some(y.oid)));
                            } else if y.mode.is_blob_or_symlink() {
                                blobs.push(y.oid);
                            }
                        }
                    }
                }
            }
            blobs.sort_unstable();
            blobs.dedup();
            count += blobs.len();
            if count > budget {
                return Err(ApiError::ServiceUnavailable(format!(
                    "diff touches more than {budget} objects; too large to render from the remote pack set"
                )));
            }
            self.fault_many(&blobs).await?;
        }
        self.local
            .refresh_async()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Fault everything `git blame --porcelain <rev> -- <path>` reads into the
    /// loose store: each commit of the path history, its path trees and every
    /// version of the file blob — bounded by BLAME_BUDGET commits. Paths that
    /// did not yet exist are boundaries (their ancestors are not followed);
    /// merge commits get every parent's path state faulted because blame diffs
    /// against all of them.
    pub async fn fault_blame(&self, commit_sha: &str, path: &str) -> Result<(), ApiError> {
        let start = ObjectId::from_hex(commit_sha.as_bytes())
            .map_err(|_| not_found(format!("invalid commit sha {commit_sha}")))?;
        #[derive(PartialEq, Eq)]
        struct Item(i64, u64, ObjectId);
        impl Ord for Item {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                self.0.cmp(&o.0).then_with(|| o.1.cmp(&self.1))
            }
        }
        impl PartialOrd for Item {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        let mut heap = BinaryHeap::new();
        let mut seen: HashSet<ObjectId> = HashSet::new();
        let mut seq = 0u64;
        let start_meta = self.commit(&start).await?;
        heap.push(Item(start_meta.commit_time, seq, start));
        seen.insert(start);
        let budget = test_budget("WALGIT_TEST_BLAME_BUDGET", BLAME_BUDGET);
        let mut visited = 0usize;
        while let Some(Item(_, _, oid)) = heap.pop() {
            visited += 1;
            if visited > budget {
                self.reporter
                    .notice(format!("blame: gave up after {budget} commits"));
                return Err(ApiError::ServiceUnavailable(format!(
                    "path '{path}' has more than {budget} commits of ancestry; too deep to blame from the remote pack set — use a host with the packs or a bundle"
                )));
            }
            if visited.is_multiple_of(100) {
                self.reporter.bar(
                    "Reading blame history".to_string(),
                    visited as u64,
                    None,
                    "commits",
                );
            }
            let (meta, target, mode) = match self.fault_path(&oid, path).await {
                Ok(x) => x,
                Err(ApiError::NotFound(_)) => continue, // path absent: boundary
                Err(e) => return Err(e),
            };
            if mode.is_blob_or_symlink() {
                self.fault(&target).await?;
            }
            for par in meta.parents {
                if seen.insert(par) {
                    // Fault the parent's path state now (idempotent with its own
                    // pop) so merge diffs have every parent's tree and blob.
                    match self.fault_path(&par, path).await {
                        Ok((_, ptarget, pmode)) => {
                            if pmode.is_blob_or_symlink() {
                                self.fault(&ptarget).await?;
                            }
                        }
                        // Path absent in this parent: a boundary, not an error.
                        Err(ApiError::NotFound(_)) => {}
                        // Real failures must surface, or the later `git blame`
                        // fails with an unreproducible local error.
                        Err(e) => return Err(e),
                    }
                    seq += 1;
                    let pm = self.commit(&par).await?;
                    heap.push(Item(pm.commit_time, seq, par));
                }
            }
        }
        self.local
            .refresh_async()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Fault every tree and blob reachable from `commit` into the loose store
    /// so an unmodified `git archive` can run against them — level-parallel,
    /// deduped, bounded by `ARCHIVE_OBJECT_BUDGET` objects (503 past it: a tree
    /// this large should be downloaded as a static bundle instead).
    pub async fn fault_tree_all(&self, commit_sha: &str) -> Result<(), ApiError> {
        let start = ObjectId::from_hex(commit_sha.as_bytes())
            .map_err(|_| not_found(format!("invalid commit sha {commit_sha}")))?;
        let c = self.commit(&start).await?;
        self.fault(&start).await?;
        let mut frontier: Vec<ObjectId> = vec![c.tree];
        let mut seen: HashSet<ObjectId> = HashSet::new();
        seen.insert(c.tree);
        let budget = test_budget("WALGIT_TEST_ARCHIVE_BUDGET", ARCHIVE_OBJECT_BUDGET);
        let mut count = 0usize;
        let too_large = || {
            ApiError::ServiceUnavailable(format!(
                "tree at {commit_sha} has more than {budget} objects; too large to archive from the remote pack set — clone via bundle-uri or use a host with the packs"
            ))
        };
        while !frontier.is_empty() {
            count += frontier.len();
            if count > budget {
                return Err(too_large());
            }
            self.fault_many(&frontier).await?;
            let mut next: Vec<ObjectId> = Vec::new();
            let mut blobs: Vec<ObjectId> = Vec::new();
            for tree in &frontier {
                for e in self.tree_entries(tree).await? {
                    if e.mode.is_tree() {
                        if seen.insert(e.oid) {
                            next.push(e.oid);
                        }
                    } else if e.mode.is_blob_or_symlink() && seen.insert(e.oid) {
                        blobs.push(e.oid);
                    }
                }
            }
            count += blobs.len();
            if count > budget {
                return Err(too_large());
            }
            self.fault_many(&blobs).await?;
            frontier = next;
        }
        self.local
            .refresh_async()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(())
    }
}

/// git's tree entry ordering: names compared as if trees had a trailing '/'.
fn tree_cmp(a: &[u8], a_tree: bool, b: &[u8], b_tree: bool) -> std::cmp::Ordering {
    let n = a.len().min(b.len());
    match a[..n].cmp(&b[..n]) {
        std::cmp::Ordering::Equal => {}
        o => return o,
    }
    let ca = a.get(n).copied().or(if a_tree { Some(b'/') } else { None });
    let cb = b.get(n).copied().or(if b_tree { Some(b'/') } else { None });
    ca.cmp(&cb)
}

pub fn parse_tree(data: &[u8], hash: gix_hash::Kind) -> Result<Vec<TreeEntryRef>, ApiError> {
    let mut out = Vec::new();
    for e in gix_object::TreeRefIter::from_bytes(data, hash) {
        let e = e.map_err(|e| ApiError::Internal(format!("parse tree: {e}")))?;
        out.push(TreeEntryRef {
            name: e.filename.to_vec(),
            oid: e.oid.to_owned(),
            mode: e.mode,
        });
    }
    Ok(out)
}

fn fmt_time(sig: &gix_actor::SignatureRef<'_>) -> (String, i64) {
    match sig.time() {
        Ok(t) => (
            t.format_or_unix(gix_date::time::format::ISO8601_STRICT),
            t.seconds,
        ),
        Err(_) => (String::new(), 0),
    }
}

pub fn parse_commit(
    id: ObjectId,
    data: &[u8],
    hash: gix_hash::Kind,
) -> Result<CommitMeta, ApiError> {
    let c = gix_object::CommitRef::from_bytes(data, hash)
        .map_err(|e| ApiError::Internal(format!("parse commit {id}: {e}")))?;
    let author = c
        .author()
        .map_err(|e| ApiError::Internal(format!("parse author {id}: {e}")))?;
    let committer = c
        .committer()
        .map_err(|e| ApiError::Internal(format!("parse committer {id}: {e}")))?;
    let (author_date, _) = fmt_time(&author);
    let (commit_date, commit_time) = fmt_time(&committer);
    let msg = c.message();
    let subject = msg.summary().to_string();
    let body = msg
        .body()
        .map(|b| b.to_string())
        .unwrap_or_default()
        .trim()
        .to_string();
    Ok(CommitMeta {
        id,
        tree: c.tree(),
        parents: c.parents().collect(),
        author: author.name.to_string(),
        author_email: author.email.to_string(),
        author_date,
        committer: committer.name.to_string(),
        commit_date,
        commit_time,
        subject,
        body,
    })
}
