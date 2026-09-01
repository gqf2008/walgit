//! D1 collaboration protocol (`docs/D1_COLLAB_DESIGN.md` §4.2/§4.3): the
//! deterministic aggregation over `refs/collab/*` — entry schema, canonical
//! form, Ed25519 verification, `thread` / `pr` / `merge_rule_eval` / `report`.
//! Pure functions: every client that reads the same refs and verifies the same
//! signatures computes the same answer. Shared by the `walgit collab` CLI and
//! the server's collab API.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

// ---- §4.2 entry schema -------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub version: u32,
    pub kind: String,
    pub id: String,
    pub actor: String,
    pub ts: i64,
    /// Previous entry's object id in the thread, or "" for the root.
    pub parent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<EntryRefs>,
    pub body: serde_json::Value,
    /// `ed25519:<base64>` over the canonical form of the entry without `sig`.
    #[serde(default)]
    pub sig: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EntryRefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

/// A parsed entry together with its object id and the principal whose inbox
/// holds it.
#[derive(Clone, Debug)]
pub struct EntryRef {
    pub oid: String,
    pub principal: String,
    pub entry: Entry,
}

impl EntryRef {
    /// Whether the entry counts as verified: the signature checks against the
    /// actor's registered key **and** the inbox it was found in names the
    /// entry's own principal. The inbox model (D1 §4.1) shards write access by
    /// principal; a policy that lets anyone write any inbox must not smuggle
    /// an entry across principals — the signature alone only proves the actor
    /// signed it, not that it belongs in this inbox.
    pub fn is_verified(&self, principals: &HashMap<String, String, impl std::hash::BuildHasher>) -> bool {
        self.principal == self.entry.actor
            && principals
                .get(&self.entry.actor)
                .is_some_and(|k| verify_entry(&self.entry, k).is_ok())
    }
}

// ---- §4.2 canonical form (the signed bytes) ----------------------------------

/// Recursive key-sorted, whitespace-free JSON: objects sorted by key bytes,
/// arrays in order, strings JSON-escaped, numbers as JSON numbers. This must
/// match the SDK's `canonicalize` (web/sdk/repos.ts) so cross-language
/// verifiers reproduce the exact signed input.
pub fn canonicalize(value: &serde_json::Value) -> String {
    let mut out = String::new();
    canonicalize_into(value, &mut out);
    out
}

fn canonicalize_into(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        serde_json::Value::String(s) => {
            out.push_str(&serde_json::to_string(s).unwrap_or_default());
        }
        serde_json::Value::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonicalize_into(v, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(o) => {
            out.push('{');
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_unstable();
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                canonicalize_into(&o[*k], out);
            }
            out.push('}');
        }
    }
}

/// The canonical bytes an entry's signature covers: the entry without `sig`.
pub fn entry_canonical(entry: &Entry) -> String {
    let mut unsigned = entry.clone();
    unsigned.sig.clear();
    let value = serde_json::to_value(unsigned).unwrap_or_default();
    canonicalize(&value)
}

// ---- verification ------------------------------------------------------------

/// Verify an entry against its principal's public key (base64) from
/// `refs/collab/meta/principals/<principal>`.
pub fn verify_entry(entry: &Entry, public_key_b64: &str) -> Result<(), String> {
    let sig_b64 = entry
        .sig
        .strip_prefix("ed25519:")
        .ok_or_else(|| "sig must be `ed25519:<base64>`".to_string())?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sig_b64)
        .map_err(|e| format!("bad sig base64: {e}"))?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|e| format!("bad signature: {e}"))?;
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64.trim())
        .map_err(|e| format!("bad public key base64: {e}"))?;
    let key_bytes: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| "public key must be 32 raw Ed25519 bytes".to_string())?;
    let vk = VerifyingKey::from_bytes(&key_bytes).map_err(|e| format!("bad public key: {e}"))?;
    vk.verify_strict(entry_canonical(entry).as_bytes(), &sig)
        .map_err(|e| format!("signature verification failed: {e}"))
}

/// Sign an entry's canonical form with a `SigningKey`; returns `ed25519:<base64>`.
/// Used by the write path (`walgit collab entry` / the SDK); the aggregation
/// core only verifies, so this is exercised by tests.
#[allow(dead_code)]
pub fn sign_entry(entry: &mut Entry, key: &SigningKey) -> String {
    let canonical = entry_canonical(entry);
    let sig = key.sign(canonical.as_bytes());
    format!(
        "ed25519:{}",
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    )
}

// ---- §4.3 deterministic aggregation ------------------------------------------

/// One issue/thread: entries referencing the same `id`, topologically ordered
/// by the `parent` chain (deterministic: ts as the tie-break).
pub fn thread<'a>(entries: &[&'a EntryRef]) -> Vec<&'a EntryRef> {
    let by_oid: HashMap<&str, &EntryRef> = entries
        .iter()
        .map(|e| (e.oid.as_str(), *e))
        .collect();
    let mut emitted: HashMap<&str, bool> = HashMap::new();
    let mut out: Vec<&EntryRef> = Vec::new();
    // Deterministic first pass order: (ts, actor, oid).
    let mut pending: Vec<&EntryRef> = entries.to_vec();
    pending.sort_by(|a, b| {
        (a.entry.ts, a.entry.actor.as_str(), a.oid.as_str())
            .cmp(&(b.entry.ts, b.entry.actor.as_str(), b.oid.as_str()))
    });
    let mut guard = 0usize;
    while !pending.is_empty() && guard < pending.len() * 2 + 1 {
        guard += 1;
        let mut next: Vec<&EntryRef> = Vec::new();
        for e in pending {
            let ready = e.entry.parent.is_empty()
                || !by_oid.contains_key(e.entry.parent.as_str())
                || emitted.get(e.entry.parent.as_str()).copied().unwrap_or(false);
            if ready {
                emitted.insert(&e.oid, true);
                out.push(e);
            } else {
                next.push(e);
            }
        }
        pending = next;
    }
    out.extend(pending); // cycles or dangling parents: emit the rest deterministically
    out
}

/// The aggregated PR view for an `id` that has a `patch` entry.
#[derive(Serialize, Clone, Debug)]
pub struct PrView {
    pub id: String,
    pub base: Option<String>,
    pub head: Option<String>,
    pub status: String,
    pub reviews: Vec<Review>,
    /// Verified `approve` reviews by non-agent actors.
    pub human_approvals: Vec<Review>,
    pub unverified: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Review {
    pub actor: String,
    pub decision: String,
    pub ts: i64,
    pub oid: String,
}

fn is_agent(actor: &str) -> bool {
    actor.starts_with("svc-")
}

pub fn pr_view(
    entries: &[&EntryRef],
    principals: &HashMap<String, String, impl std::hash::BuildHasher>,
) -> PrView {
    let ordered = thread(entries);
    let mut base = None;
    let mut head = None;
    let mut status = "open".to_string();
    let mut reviews: Vec<Review> = Vec::new();
    let mut human_approvals: Vec<Review> = Vec::new();
    let mut unverified: Vec<String> = Vec::new();
    for r in &ordered {
        let e = &r.entry;
        if e.kind == "patch"
            && let Some(rs) = &e.refs
        {
            base = rs.base.clone().or(base);
            head = rs.head.clone().or(head);
        }
        let verified = r.is_verified(principals);
        if !verified {
            unverified.push(format!("{}@{}", e.actor, r.oid));
        }
        match e.kind.as_str() {
            "status" => {
                if let Some(st) = e.body.get("status").and_then(|v| v.as_str())
                    && (st == "merged" || st == "closed")
                {
                    status = st.to_string();
                }
            }
            "merge_result" => {
                if e.body
                    .get("merged")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    status = "merged".to_string();
                }
            }
            "review" => {
                let decision = e
                    .body
                    .get("decision")
                    .and_then(|v| v.as_str())
                    .unwrap_or("comment")
                    .to_string();
                let review = Review {
                    actor: e.actor.clone(),
                    decision: decision.clone(),
                    ts: e.ts,
                    oid: r.oid.clone(),
                };
                if decision == "approve" && verified {
                    human_approvals.push(review.clone());
                }
                reviews.push(review);
            }
            _ => {}
        }
    }
    PrView {
        id: ordered.first().map(|r| r.entry.id.clone()).unwrap_or_default(),
        base,
        head,
        status,
        reviews,
        human_approvals,
        unverified,
    }
}

// ---- merge rule evaluation ---------------------------------------------------

/// A minimal merge rule document (stored at `refs/collab/meta/rules` or given
/// on the CLI). D1 §6: merge rules are deterministic functions of the log.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MergeRules {
    /// Ref patterns whose merges require human approvals (e.g.
    /// `["refs/heads/main"]`). Empty = nothing protected.
    #[serde(default)]
    pub protect: Vec<String>,
    #[serde(default = "default_approvals")]
    pub require_human_approvals: usize,
}

fn default_approvals() -> usize {
    1
}

#[derive(Serialize, Clone, Debug)]
pub struct MergeEval {
    pub allowed: bool,
    pub reason: String,
    pub satisfied_by: Vec<String>,
}

/// Evaluate whether the PR may be merged: protected bases need at least
/// `require_human_approvals` verified approvals from non-agent actors.
pub fn merge_rule_eval(rules: &MergeRules, pr: &PrView) -> MergeEval {
    let protected = pr
        .base
        .as_deref()
        .is_some_and(|b| rules.protect.iter().any(|p| p == b || b.starts_with(p)));
    if !protected {
        return MergeEval {
            allowed: true,
            reason: "base is not protected".to_string(),
            satisfied_by: Vec::new(),
        };
    }
    let approvals: Vec<&Review> = pr
        .human_approvals
        .iter()
        .filter(|r| r.decision == "approve" && !is_agent(&r.actor))
        .collect();
    let satisfied_by: Vec<String> = approvals.iter().map(|r| r.actor.clone()).collect();
    if approvals.len() >= rules.require_human_approvals {
        MergeEval {
            allowed: true,
            reason: format!("{} human approval(s) on protected base", approvals.len()),
            satisfied_by,
        }
    } else {
        MergeEval {
            allowed: false,
            reason: format!(
                "protected base needs {} human approval(s), got {}",
                rules.require_human_approvals,
                approvals.len()
            ),
            satisfied_by,
        }
    }
}

// ---- report: read-only observability dashboard (D1 §8) -------------------------

#[derive(Serialize, Clone, Debug)]
pub struct ReportThread {
    pub id: String,
    pub entries: usize,
    pub verified: usize,
    pub last_ts: i64,
    pub kinds: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ReportPr {
    pub id: String,
    pub base: Option<String>,
    pub head: Option<String>,
    pub status: String,
    pub approvals: usize,
    pub merge_allowed: bool,
    pub merge_reason: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Report {
    pub threads: Vec<ReportThread>,
    pub prs: Vec<ReportPr>,
    pub total_entries: usize,
    pub verified_entries: usize,
    pub unverified_entries: usize,
    pub missing_principals: usize,
    pub by_actor: Vec<(String, usize)>,
    pub by_kind: Vec<(String, usize)>,
}

pub fn build_report(
    entries: &[&EntryRef],
    principals: &HashMap<String, String, impl std::hash::BuildHasher>,
    rules: &MergeRules,
) -> Report {
    let mut by_thread: BTreeMap<&str, Vec<&EntryRef>> = BTreeMap::new();
    for e in entries {
        by_thread.entry(e.entry.id.as_str()).or_default().push(e);
    }
    let mut report = Report::default();
    for (id, group) in &by_thread {
        let ordered = thread(group);
        let verified = ordered.iter().filter(|r| r.is_verified(principals)).count();
        let mut kinds: Vec<String> = group.iter().map(|r| r.entry.kind.clone()).collect();
        kinds.sort();
        kinds.dedup();
        report.threads.push(ReportThread {
            id: (*id).to_string(),
            entries: group.len(),
            verified,
            last_ts: group.iter().map(|r| r.entry.ts).max().unwrap_or(0),
            kinds,
        });
    }
    for (id, group) in &by_thread {
        if group.iter().any(|r| r.entry.kind == "patch") {
            let pr = pr_view(group, principals);
            let eval = merge_rule_eval(rules, &pr);
            report.prs.push(ReportPr {
                id: (*id).to_string(),
                base: pr.base.clone(),
                head: pr.head.clone(),
                status: pr.status.clone(),
                approvals: pr.human_approvals.len(),
                merge_allowed: eval.allowed,
                merge_reason: eval.reason.clone(),
            });
        }
    }
    report.total_entries = entries.len();
    for r in entries {
        let verified = r.is_verified(principals);
        if verified {
            report.verified_entries += 1;
        } else {
            report.unverified_entries += 1;
            if !principals.contains_key(&r.entry.actor) {
                report.missing_principals += 1;
            }
        }
    }
    let mut by_actor: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for r in entries {
        *by_actor.entry(&r.entry.actor).or_default() += 1;
        *by_kind.entry(&r.entry.kind).or_default() += 1;
    }
    report.by_actor = by_actor.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    report.by_kind = by_kind.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    report
}

// ---- board: a deterministic projection of the threads (D1 §8) -----------------
//
// The board is not state and not a view with its own write path: it is the
// thread set folded under a declarative column definition versioned with the
// repository (`.walgit/board.toml`, plain git — not a collab ref, so editing it
// is an ordinary commit reviewed like any other). Moving a card is an ordinary
// signed `status` entry; the projection re-derives the columns from it.

/// Where the board definition is versioned (repo-relative, in the tree).
pub const BOARD_PATH: &str = ".walgit/board.toml";

/// One lane's predicate. A card enters the **first** declared column whose
/// predicate it satisfies; empty predicate fields match anything, so a column
/// with none of them set is the catch-all. Cards matching no column are not on
/// the board — say what you want to see, don't get what you didn't ask for.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoardColumnDef {
    pub name: String,
    /// The thread must contain an entry of this kind.
    #[serde(default)]
    pub kind: String,
    /// The card's effective status (`card_status`) must equal this.
    #[serde(default)]
    pub status: String,
    /// `allowed` | `blocked`: the merge-rule verdict on the thread's patch.
    /// Cards without a `patch` match neither value (there is no verdict).
    #[serde(default)]
    pub merge: String,
    /// Only cards carrying at least one unverified entry.
    #[serde(default)]
    pub unverified: bool,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BoardSortBy {
    /// Last activity (`last_ts`) — the default.
    #[default]
    Ts,
    /// Thread id.
    Id,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BoardSortDirection {
    /// Newest first — the default.
    #[default]
    Desc,
    Asc,
}

/// Stable order inside a column. The sort field carries the configured
/// direction; the card id breaks ties ascending either way, so the output is a
/// total order over the input (same refs ⇒ byte-identical board, any order the
/// refs were read in).
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoardSort {
    #[serde(default)]
    pub by: BoardSortBy,
    #[serde(default)]
    pub direction: BoardSortDirection,
}

/// The board definition: `version` + column predicates + a sort. Kept
/// deliberately minimal — a board nobody can re-derive by hand is a second
/// state source, not a projection.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoardDef {
    pub version: u32,
    #[serde(default)]
    pub sort: BoardSort,
    /// `[[column]]` in the document, in declaration order (first match wins).
    #[serde(default, rename = "column")]
    pub columns: Vec<BoardColumnDef>,
}

impl BoardDef {
    /// Fails closed: an unsupported version, no columns, an empty/duplicate
    /// column name or an unknown `merge` verdict is a broken board — a client
    /// must show the error, not silently fold cards into the wrong lane.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "unsupported board version {} (only 1 exists)",
                self.version
            ));
        }
        if self.columns.is_empty() {
            return Err("a board needs at least one column".to_string());
        }
        for c in &self.columns {
            if c.name.is_empty() {
                return Err("column name must not be empty".to_string());
            }
            if !c.merge.is_empty() && c.merge != "allowed" && c.merge != "blocked" {
                return Err(format!(
                    "column {:?}: merge must be \"allowed\" or \"blocked\"",
                    c.name
                ));
            }
        }
        let mut names: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() != self.columns.len() {
            return Err("duplicate column name".to_string());
        }
        Ok(())
    }
}

/// Parse + validate a board definition document.
pub fn parse_board_def(doc: &str) -> Result<BoardDef, String> {
    let def: BoardDef = toml::from_str(doc).map_err(|e| e.to_string())?;
    def.validate()?;
    Ok(def)
}

/// The board when the repository defines none: one lane per well-known status
/// plus a catch-all. Deterministic like any other projection input.
pub fn default_board() -> BoardDef {
    BoardDef {
        version: 1,
        sort: BoardSort::default(),
        columns: vec![
            BoardColumnDef {
                name: "open".to_string(),
                status: "open".to_string(),
                ..BoardColumnDef::default()
            },
            BoardColumnDef {
                name: "merged".to_string(),
                status: "merged".to_string(),
                ..BoardColumnDef::default()
            },
            BoardColumnDef {
                name: "closed".to_string(),
                status: "closed".to_string(),
                ..BoardColumnDef::default()
            },
            BoardColumnDef {
                name: "other".to_string(),
                ..BoardColumnDef::default()
            },
        ],
    }
}

/// One card: what the thread aggregation says about a work unit, flattened for
/// the three renderers (CLI, endpoint, SPA). Field order here is the JSON wire
/// order — every client serializes this exact struct, which is what makes the
/// byte-equality acceptance test meaningful.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BoardCard {
    pub id: String,
    /// The root entry's `body.title`, "" when it has none.
    pub title: String,
    pub actor: String,
    /// Effective work-unit status (see `card_status`).
    pub status: String,
    pub created_ts: i64,
    pub last_ts: i64,
    pub entries: usize,
    pub verified: usize,
    pub unverified: usize,
    /// Distinct entry kinds in the thread, sorted.
    pub kinds: Vec<String>,
    /// Tip of the parent chain — the `parent` a follow-up entry chains to (the
    /// board page's move posts its `status` entry on it).
    pub last_oid: String,
    /// Merge-rule verdict when the thread has a patch, else `null`.
    pub merge: Option<BoardMergeCard>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BoardMergeCard {
    pub allowed: bool,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BoardColumn {
    pub name: String,
    pub cards: Vec<BoardCard>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Board {
    pub columns: Vec<BoardColumn>,
}

/// The card's effective status: entries are applied in thread order and the
/// last match wins — every `status` entry sets its `body.status`, a
/// `merge_result` with `merged = true` sets "merged" (a later `status` entry
/// still overrides it); default "open". Deliberately wider than `pr_view`'s
/// open/merged/closed state machine: the board tracks the work unit (§4.2
/// names in-progress / needs-review / blocked / needs-human), the PR view
/// tracks merge state.
fn card_status(ordered: &[&EntryRef]) -> String {
    let mut status = "open".to_string();
    for r in ordered {
        match r.entry.kind.as_str() {
            "status" => {
                if let Some(st) = r.entry.body.get("status").and_then(|v| v.as_str()) {
                    status = st.to_string();
                }
            }
            "merge_result"
                if r.entry
                    .body
                    .get("merged")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true) =>
            {
                status = "merged".to_string();
            }
            _ => {}
        }
    }
    status
}

/// First-match-wins against the column predicate (D1 §8).
fn card_matches(card: &BoardCard, col: &BoardColumnDef) -> bool {
    if !col.kind.is_empty() && !card.kinds.iter().any(|k| k == &col.kind) {
        return false;
    }
    if !col.status.is_empty() && card.status != col.status {
        return false;
    }
    if !col.merge.is_empty() {
        let hit = match (&card.merge, col.merge.as_str()) {
            (Some(m), "allowed") => m.allowed,
            (Some(m), "blocked") => !m.allowed,
            _ => false,
        };
        if !hit {
            return false;
        }
    }
    !(col.unverified && card.unverified == 0)
}

fn sort_cards(cards: &mut [BoardCard], sort: BoardSort) {
    cards.sort_by(|a, b| {
        let field = match sort.by {
            BoardSortBy::Ts => a.last_ts.cmp(&b.last_ts),
            BoardSortBy::Id => a.id.cmp(&b.id),
        };
        let field = match sort.direction {
            BoardSortDirection::Asc => field,
            BoardSortDirection::Desc => field.reverse(),
        };
        field.then_with(|| a.id.cmp(&b.id))
    });
}

/// The board projection: a pure function `(entries, principals, rules, board
/// definition) → columns`. Threads group by entry `id` exactly as
/// `build_report` does; each becomes at most one card in the first column whose
/// predicate it satisfies, sorted by the definition's sort. Two clients reading
/// the same refs compute byte-identical boards — that property is the e2e
/// acceptance (CLI offline aggregation vs the server endpoint).
pub fn build_board(
    entries: &[&EntryRef],
    principals: &HashMap<String, String, impl std::hash::BuildHasher>,
    rules: &MergeRules,
    board: &BoardDef,
) -> Board {
    let mut by_thread: BTreeMap<&str, Vec<&EntryRef>> = BTreeMap::new();
    for e in entries {
        by_thread.entry(e.entry.id.as_str()).or_default().push(e);
    }
    let mut columns: Vec<(BoardColumnDef, BoardColumn)> = board
        .columns
        .iter()
        .map(|def| {
            (
                def.clone(),
                BoardColumn {
                    name: def.name.clone(),
                    cards: Vec::new(),
                },
            )
        })
        .collect();
    for (id, group) in &by_thread {
        let ordered = thread(group);
        let verified = ordered.iter().filter(|r| r.is_verified(principals)).count();
        let mut kinds: Vec<String> = group.iter().map(|r| r.entry.kind.clone()).collect();
        kinds.sort();
        kinds.dedup();
        let root = ordered.first();
        let merge = group.iter().any(|r| r.entry.kind == "patch").then(|| {
            let pr = pr_view(group, principals);
            let eval = merge_rule_eval(rules, &pr);
            BoardMergeCard {
                allowed: eval.allowed,
                reason: eval.reason,
            }
        });
        let card = BoardCard {
            id: (*id).to_string(),
            title: root
                .and_then(|r| r.entry.body.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            actor: root.map(|r| r.entry.actor.clone()).unwrap_or_default(),
            status: card_status(&ordered),
            created_ts: root.map_or(0, |r| r.entry.ts),
            last_ts: group.iter().map(|r| r.entry.ts).max().unwrap_or(0),
            entries: group.len(),
            verified,
            unverified: group.len() - verified,
            kinds,
            last_oid: ordered.last().map(|r| r.oid.clone()).unwrap_or_default(),
            merge,
        };
        if let Some((_, col)) = columns.iter_mut().find(|(def, _)| card_matches(&card, def)) {
            col.cards.push(card);
        }
    }
    for (_, col) in &mut columns {
        sort_cards(&mut col.cards, board.sort);
    }
    Board {
        columns: columns.into_iter().map(|(_, col)| col).collect(),
    }
}

#[cfg(test)]
mod board_tests {
    use super::*;
    use std::hash::{Hash, Hasher};

    fn keypair() -> (SigningKey, String) {
        let sk = SigningKey::from_bytes(&[11u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        (sk, base64::engine::general_purpose::STANDARD.encode(pk))
    }

    fn signed(sk: &SigningKey, e: &mut Entry) {
        e.sig = sign_entry(e, sk);
    }

    fn entry(id: &str, kind: &str, actor: &str, parent: &str, ts: i64, body: serde_json::Value) -> Entry {
        Entry {
            version: 1,
            kind: kind.into(),
            id: id.into(),
            actor: actor.into(),
            ts,
            parent: parent.into(),
            refs: None,
            body,
            sig: String::new(),
        }
    }

    /// The content-derived oid `refs_of` assigns (a stand-in for the real blob
    /// sha): permuting the input must not change any card's oid, which feeds
    /// `last_oid`, the thread order tie-break and the merge evaluation.
    fn test_oid(e: &Entry) -> String {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(e).unwrap_or_default().hash(&mut h);
        format!("oid{:016x}", h.finish())
    }

    fn refs_of(entries: &[Entry]) -> Vec<EntryRef> {
        entries
            .iter()
            .map(|e| EntryRef {
                oid: test_oid(e),
                principal: e.actor.clone(),
                entry: e.clone(),
            })
            .collect()
    }

    fn column_of<'a>(board: &'a Board, name: &str) -> &'a BoardColumn {
        board
            .columns
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no column {name}"))
    }

    #[test]
    fn board_toml_parses_and_validates() {
        let doc = r#"
version = 1

[sort]
by = "ts"
direction = "desc"

[[column]]
name = "review"
status = "needs-review"

[[column]]
name = "mergeable"
kind = "patch"
merge = "allowed"

[[column]]
name = "suspect"
unverified = true

[[column]]
name = "everything else"
"#;
        let def = parse_board_def(doc).expect("parses");
        assert_eq!(def.columns.len(), 4);
        assert_eq!(def.sort.by, BoardSortBy::Ts);
        assert_eq!(def.sort.direction, BoardSortDirection::Desc);
        assert_eq!(def.columns[1].kind, "patch");
        assert!(def.columns[2].unverified);

        // Fails closed on a broken definition — every one of these must error,
        // not silently fold cards into a wrong lane.
        assert!(parse_board_def("version = 2\n[[column]]\nname = \"x\"\n").is_err(), "unknown version");
        assert!(parse_board_def("[sort]\n[[column]]\nname = \"x\"\n").is_err(), "version required");
        assert!(parse_board_def("version = 1\n").is_err(), "no columns");
        assert!(parse_board_def("version = 1\n[[column]]\nname = \"\"\n").is_err(), "empty name");
        assert!(
            parse_board_def("version = 1\n[[column]]\nname = \"a\"\n[[column]]\nname = \"a\"\n").is_err(),
            "duplicate names"
        );
        assert!(
            parse_board_def("version = 1\n[[column]]\nname = \"a\"\nmerge = \"maybe\"\n").is_err(),
            "unknown merge verdict"
        );
        assert!(
            parse_board_def("version = 1\n[[column]]\nname = \"a\"\nstatuz = \"open\"\n").is_err(),
            "a typo must not silently no-op (deny_unknown_fields)"
        );
    }

    #[test]
    fn same_refs_project_to_identical_bytes_regardless_of_read_order() {
        let (sk, pk) = keypair();
        let mut principals = HashMap::new();
        principals.insert("alice".to_string(), pk.clone());
        principals.insert("bob".to_string(), pk);

        // Specified with symbolic keys; parents resolve to the content-derived
        // oids below, exactly like a real thread chains on blob shas.
        let mut patch = entry("t1", "patch", "alice", "a4", 5, serde_json::json!({"message": "the change"}));
        patch.refs = Some(EntryRefs {
            base: Some("refs/heads/main".into()),
            head: Some("refs/heads/topic".into()),
        });
        let specs: Vec<(&str, Entry)> = vec![
            ("a1", entry("t1", "issue", "alice", "", 1, serde_json::json!({"title": "add thing"}))),
            ("a2", entry("t1", "comment", "bob", "a1", 2, serde_json::json!({"text": "looks right"}))),
            ("a3", entry("t1", "status", "alice", "a2", 3, serde_json::json!({"status": "needs-review"}))),
            ("a4", entry("t1", "review", "bob", "a3", 4, serde_json::json!({"decision": "approve"}))),
            ("a5", patch),
            // t2/t3: unsigned entries stay unverified.
            ("b1", entry("t2", "issue", "bob", "", 5, serde_json::json!({"title": "other thing"}))),
            ("c1", entry("t3", "issue", "alice", "", 6, serde_json::json!({"title": "third"}))),
            ("c2", entry("t3", "status", "alice", "c1", 7, serde_json::json!({"status": "closed"}))),
        ];
        // Content addressing, bottom-up like git: sign the entry, resolve its
        // parent to the already-computed oid, then hash the final bytes.
        // Signed: a1 (issue), a3 (status), a4 (bob's approve review — the
        // human approval the merge rule needs), a5 (patch). a2 stays unsigned
        // so the thread carries an unverified entry.
        let mut oid_of: HashMap<String, String> = HashMap::new();
        let mut es: Vec<Entry> = Vec::new();
        for (k, mut e) in specs {
            // Parent first: the signature covers the entry's final bytes,
            // parent pointer included.
            if let Some(p) = oid_of.get(&e.parent) {
                let p = p.clone();
                e.parent = p;
            } else if !e.parent.is_empty() {
                panic!("spec parent unknown");
            }
            if matches!(k, "a1" | "a3" | "a4" | "a5") {
                let sig = sign_entry(&mut e, &sk);
                e.sig = sig;
            }
            oid_of.insert(k.to_string(), test_oid(&e));
            es.push(e);
        }
        let rules = MergeRules {
            protect: vec!["refs/heads/main".into()],
            require_human_approvals: 1,
        };
        let board_def = parse_board_def(
            "version = 1\n[[column]]\nname = \"review\"\nstatus = \"needs-review\"\n\
             [[column]]\nname = \"done\"\nstatus = \"closed\"\n\
             [[column]]\nname = \"suspect\"\nunverified = true\n\
             [[column]]\nname = \"mergeable\"\nkind = \"patch\"\nmerge = \"allowed\"\n\
             [[column]]\nname = \"everything else\"\n",
        )
        .expect("board def");

        let project = |list: &[Entry]| {
            let refs = refs_of(list);
            let borrowed: Vec<&EntryRef> = refs.iter().collect();
            serde_json::to_vec(&build_board(&borrowed, &principals, &rules, &board_def))
                .expect("serialize")
        };
        let bytes = project(&es);
        // Same input, again: identical bytes.
        assert_eq!(bytes, project(&es));
        // Refs read in a different order (fetch/clone timing, page boundaries):
        // still byte-identical — the projection is a function of the set.
        let mut permuted = es.clone();
        permuted.reverse();
        assert_eq!(bytes, project(&permuted));

        let board: Board = serde_json::from_slice(&bytes).expect("board json");
        let t1 = &column_of(&board, "review").cards[0];
        // First match wins: t1 (needs-review AND patch-mergeable AND mostly
        // verified) lands in "review", not in a later matching lane.
        assert_eq!(column_of(&board, "review").cards.len(), 1);
        assert_eq!(t1.id, "t1");
        assert_eq!(t1.status, "needs-review");
        assert_eq!(t1.last_oid, oid_of["a5"], "tip of the parent chain");
        assert_eq!(t1.verified, 4);
        assert_eq!(t1.unverified, 1, "bob's unsigned comment");
        assert_eq!(t1.actor, "alice");
        assert_eq!(t1.created_ts, 1);
        assert_eq!(t1.last_ts, 5, "max ts over the thread");
        assert!(t1.merge.as_ref().expect("patch verdict").allowed);
        assert_eq!(column_of(&board, "done").cards[0].id, "t3");
        assert_eq!(column_of(&board, "suspect").cards[0].id, "t2");
        assert_eq!(column_of(&board, "suspect").cards[0].unverified, 1);
        assert!(column_of(&board, "mergeable").cards.is_empty(), "first match wins");
        assert!(column_of(&board, "everything else").cards.is_empty());
    }

    #[test]
    fn status_entries_move_cards_and_the_default_board_covers_every_status() {
        let (sk, pk) = keypair();
        let mut principals = HashMap::new();
        principals.insert("alice".to_string(), pk);
        let mut es = vec![
            entry("t1", "issue", "alice", "", 1, serde_json::json!({"title": "move me"})),
            entry("t1", "status", "alice", "a1", 2, serde_json::json!({"status": "in-progress"})),
        ];
        signed(&sk, &mut es[0]);
        signed(&sk, &mut es[1]);
        let refs = refs_of(&es);
        let borrowed: Vec<&EntryRef> = refs.iter().collect();
        let rules = MergeRules::default();
        let def = default_board();
        def.validate().expect("default board is valid");

        let board = build_board(&borrowed, &principals, &rules, &def);
        assert_eq!(column_of(&board, "open").cards.len(), 0, "status moved it out of open");
        assert_eq!(
            column_of(&board, "other").cards.len(),
            1,
            "statuses without a lane fall to the catch-all"
        );
        assert_eq!(column_of(&board, "other").cards[0].status, "in-progress");

        // The move: one more signed `status` entry — the only write the board
        // ever needs — and the card is in "merged" for the next projection.
        let mut done = entry("t1", "status", "alice", "a2", 3, serde_json::json!({"status": "merged"}));
        signed(&sk, &mut done);
        let mut es2 = es.clone();
        es2.push(done);
        let refs2 = refs_of(&es2);
        let borrowed2: Vec<&EntryRef> = refs2.iter().collect();
        let board2 = build_board(&borrowed2, &principals, &rules, &def);
        assert_eq!(column_of(&board2, "merged").cards.len(), 1);
        assert_eq!(column_of(&board2, "merged").cards[0].id, "t1");
        // Newest first within a column; the id breaks ties. Both threads sit
        // in the catch-all (neither is "open"), both last active at ts 2.
        let mut tie = es.clone();
        let mut t9_issue = entry("t9", "issue", "alice", "", 1, serde_json::json!({"title": "tie"}));
        signed(&sk, &mut t9_issue);
        let mut t9_status = entry("t9", "status", "alice", "", 2, serde_json::json!({"status": "in-progress"}));
        signed(&sk, &mut t9_status);
        tie.push(t9_issue);
        tie.push(t9_status);
        let refs3 = refs_of(&tie);
        let borrowed3: Vec<&EntryRef> = refs3.iter().collect();
        let board3 = build_board(&borrowed3, &principals, &rules, &def);
        let other_col: Vec<&str> = column_of(&board3, "other").cards.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(other_col, vec!["t1", "t9"], "equal ts: id ascending");
    }

    #[test]
    fn cards_matching_no_column_are_not_on_the_board() {
        let principals = HashMap::new();
        let refs = refs_of(&[entry("t1", "issue", "alice", "", 1, serde_json::json!({"title": "x"}))]);
        let borrowed: Vec<&EntryRef> = refs.iter().collect();
        let def = parse_board_def("version = 1\n[[column]]\nname = \"only blocked\"\nstatus = \"blocked\"\n").expect("def");
        let board = build_board(&borrowed, &principals, &MergeRules::default(), &def);
        assert!(board.columns.len() == 1 && board.columns[0].cards.is_empty());
    }
}
