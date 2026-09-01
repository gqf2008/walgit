import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { parsePatchFiles } from "@pierre/diffs";
import { FileDiff } from "@pierre/diffs/react";
import { api, type CollabEntryRef } from "../api";
import { useRepo } from "./RepoLayout";
import { useData } from "../data";
import { Box } from "../components/Layout";
import { CollabWriteBox } from "../components/CollabWrite";

function fmtTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString();
}

export function CollabThreadPage() {
  const { full } = useRepo();
  const id = useParams().id ?? "";
  const thread = useData(`collab:${full}:thread:${id}`, () => api.collab(full).thread(id));
  const lastOid = thread.entries[thread.entries.length - 1]?.oid ?? "";
  return (
    <>
      <div className="pad">
        <Link to={`/${full}/collab`} className="muted">← collab</Link>
      </div>
      <Box title={`Thread ${thread.id}`}>
        {thread.pr && (
          <div className="kv">
            <dt>base → head</dt>
            <dd className="mono">{thread.pr.pr.base ?? "?"} → {thread.pr.pr.head ?? "?"}</dd>
            <dt>status</dt>
            <dd>{thread.pr.pr.status}</dd>
            <dt>reviews</dt>
            <dd>
              {thread.pr.pr.reviews.map((r) => `${r.actor}:${r.decision}`).join(", ") || "—"}
            </dd>
            <dt>approvals</dt>
            <dd>{thread.pr.pr.human_approvals.length} human · {thread.pr.pr.unverified.length} unverified</dd>
            <dt>merge rule</dt>
            <dd>{thread.pr.merge.allowed ? "allowed" : thread.pr.merge.reason}</dd>
          </div>
        )}
        {thread.pr && <PrDiff full={full} base={thread.pr.pr.base} head={thread.pr.pr.head} />}
        <div className="box-header" style={{ marginTop: 8 }}>Write</div>
        <CollabWriteBox full={full} id={thread.id} parent={lastOid} />
      </Box>

      {thread.entries.map((e, i) => (
        <EntryBox key={e.oid} e={e} n={i + 1} />
      ))}
    </>
  );
}

/** The PR's base→head diff, rendered on demand (the diff can be large). */
function PrDiff({ full, base, head }: { full: string; base: string | null; head: string | null }) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<{ patch: string; error?: string } | null>(null);
  const load = async () => {
    setOpen(true);
    if (state) return;
    try {
      if (!base || !head) throw new Error("base/head missing");
      const d = await api.diff(full, base, head, "patch");
      setState({ patch: d.patch ?? "" });
    } catch (e) {
      setState({ patch: "", error: e instanceof Error ? e.message : String(e) });
    }
  };
  const files = state ? parsePatchFilesSafe(state.patch, head ?? "") : [];
  return (
    <>
      <div className="box-header" style={{ marginTop: 8 }}>
        <button className="btn small" onClick={load} aria-expanded={open}>
          {open ? "Diff" : "Show diff"} {base ?? "?"} → {head ?? "?"}
        </button>
      </div>
      {open && state && (
        <div className="pad">
          {state.error && <div className="muted" style={{ color: "var(--danger, #f85149)" }}>{state.error}</div>}
          {!state.error && files.length === 0 && <div className="muted">No file changes.</div>}
          {files.map((f, i) => (
            <div key={f.name + i} className="diff-file">
              <FileDiff fileDiff={f} options={{ diffStyle: "unified", themeType: "light", overflow: "scroll" }} />
            </div>
          ))}
        </div>
      )}
    </>
  );
}

function parsePatchFilesSafe(patch: string, sha: string) {
  if (!patch) return [];
  try {
    return parsePatchFiles(patch, sha).flatMap((p) => p.files);
  } catch (e) {
    console.error(e);
    return [];
  }
}

/** CI conclusions get the only colors on this page: green/red/amber (D1-CI §8.3). */
function ciColor(conclusion: string): string {
  return conclusion === "success" ? "var(--ok, #2ea043)"
    : conclusion === "error" ? "var(--warning, #d29922)"
    : "var(--danger, #f85149)";
}

function EntryBox({ e, n }: { e: CollabEntryRef; n: number }) {
  const kind = e.entry.kind;
  const body = e.entry.body as Record<string, unknown>;
  const ciConclusion = kind === "ci_result" ? String(body.conclusion ?? "") : "";
  const title =
    kind === "issue" ? String(body.title ?? "(issue)")
    : kind === "review" ? String(body.decision ?? "comment")
    : kind === "status" ? String(body.status ?? "status")
    : kind === "ci_claim" ? `claim ${String(body.task ?? "?")} · attempt ${String(body.attempt ?? "?")}`
    : kind === "ci_result" ? `result ${String(body.task ?? "?")} · attempt ${String(body.attempt ?? "?")}`
    : kind;
  return (
    <Box title={<span>#{n} <strong>{kind}</strong> · {e.entry.actor} · {fmtTime(e.entry.ts)} · {e.verified ? "✓ verified" : "✗ unverified"}</span>}>
      <div className="pad">
        <div className="strong">
          {title}
          {ciConclusion && (
            <span style={{ marginLeft: 8, color: ciColor(ciConclusion) }}>● {ciConclusion}</span>
          )}
        </div>
        {kind === "ci_result" && (
          <div className="mono muted">
            {String(body.ref ?? "")} @ {String(body.commit ?? "").slice(0, 8)} · exit{" "}
            {body.exit_code === null || body.exit_code === undefined ? "—" : String(body.exit_code)} · {String(body.duration_ms ?? "?")} ms · log
            sha {String(body.log_sha256 ?? "").slice(0, 12)}
          </div>
        )}
        {ciConclusion && typeof body.log_summary === "string" && body.log_summary !== "" && (
          <pre className="collab-pre">{body.log_summary}</pre>
        )}
        {!ciConclusion && <pre className="collab-pre">{JSON.stringify(e.entry.body, null, 2)}</pre>}
        {(e.entry.refs?.base || e.entry.refs?.head) && (
          <div className="mono muted">
            {e.entry.refs?.base ?? "?"} → {e.entry.refs?.head ?? "?"}
          </div>
        )}
        <div className="mono muted" style={{ wordBreak: "break-all" }}>
          {e.oid} · {e.principal} · sig {e.entry.sig.slice(0, 24)}…
        </div>
      </div>
    </Box>
  );
}
