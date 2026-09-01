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
    pub fn is_verified(&self, principals: &HashMap<String, String>) -> bool {
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
    principals: &HashMap<String, String>,
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
    principals: &HashMap<String, String>,
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
