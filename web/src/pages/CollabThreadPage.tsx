import { Link, useParams } from "react-router-dom";
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
        <div className="box-header" style={{ marginTop: 8 }}>Write</div>
        <CollabWriteBox full={full} id={thread.id} parent={lastOid} />
      </Box>

      {thread.entries.map((e, i) => (
        <EntryBox key={e.oid} e={e} n={i + 1} />
      ))}
    </>
  );
}

function EntryBox({ e, n }: { e: CollabEntryRef; n: number }) {
  const kind = e.entry.kind;
  const title =
    kind === "issue" ? String((e.entry.body as { title?: unknown }).title ?? "(issue)")
    : kind === "review" ? String((e.entry.body as { decision?: unknown }).decision ?? "comment")
    : kind === "status" ? String((e.entry.body as { status?: unknown }).status ?? "status")
    : kind;
  return (
    <Box title={<span>#{n} <strong>{kind}</strong> · {e.entry.actor} · {fmtTime(e.entry.ts)} · {e.verified ? "✓ verified" : "✗ unverified"}</span>}>
      <div className="pad">
        <div className="strong">{title}</div>
        <pre className="collab-pre">{JSON.stringify(e.entry.body, null, 2)}</pre>
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
