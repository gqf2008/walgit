import { useState } from "react";
import { Link } from "react-router-dom";
import { api, type CollabReport } from "../api";
import { useRepo } from "./RepoLayout";
import { useData } from "../data";
import { Box } from "../components/Layout";
import { CollabWriteBox } from "../components/CollabWrite";

function fmtTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString();
}

function uuid(): string {
  return typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

export function CollabPage() {
  const { full } = useRepo();
  const report = useData(`collab:${full}:report`, () => api.collab(full).report());
  return <CollabView full={full} report={report} />;
}

function CollabView({ full, report }: { full: string; report: CollabReport }) {
  const [newId, setNewId] = useState(() => uuid());
  return (
    <>
      <Box title="D1 collaboration">
        <div className="kv">
          <dt>Threads</dt>
          <dd>{report.threads.length}</dd>
          <dt>Pull requests</dt>
          <dd>{report.prs.length}</dd>
          <dt>Entries</dt>
          <dd>
            {report.total_entries} · <span className="ok">✓ {report.verified_entries} verified</span> ·{" "}
            <span className="muted">{report.unverified_entries} unverified</span> · {report.missing_principals} missing keys
          </dd>
          <dt>Board</dt>
          <dd>
            <Link to={`/${full}/collab/board`}>work-unit board →</Link>
          </dd>
        </div>
        <div className="box-header" style={{ marginTop: 8 }}>New thread</div>
        <CollabWriteBox full={full} id={newId} parent="" onPosted={() => setNewId(uuid())} />
      </Box>

      <Box title="Threads">
        {report.threads.length === 0 && <div className="pad muted">No threads yet — post the first entry above.</div>}
        {report.threads.length > 0 && (
          <table className="grid">
            <thead>
              <tr>
                <th>id</th>
                <th>kinds</th>
                <th>entries</th>
                <th>verified</th>
                <th>last activity</th>
              </tr>
            </thead>
            <tbody>
              {report.threads.map((t) => (
                <tr key={t.id}>
                  <td>
                    <Link to={`/${full}/collab/thread/${encodeURIComponent(t.id)}`} className="strong">
                      {t.id}
                    </Link>
                  </td>
                  <td>{t.kinds.join(", ")}</td>
                  <td>{t.entries}</td>
                  <td>{t.verified}</td>
                  <td>{fmtTime(t.last_ts)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Box>

      <Box title="Pull requests">
        {report.prs.length === 0 && <div className="pad muted">No patches.</div>}
        {report.prs.length > 0 && (
          <table className="grid">
            <thead>
              <tr>
                <th>id</th>
                <th>base → head</th>
                <th>status</th>
                <th>approvals</th>
                <th>merge</th>
              </tr>
            </thead>
            <tbody>
              {report.prs.map((p) => (
                <tr key={p.id}>
                  <td>
                    <Link to={`/${full}/collab/thread/${encodeURIComponent(p.id)}`} className="strong">
                      {p.id}
                    </Link>
                  </td>
                  <td className="mono">{p.base ?? "?"} → {p.head ?? "?"}</td>
                  <td>{p.status}</td>
                  <td>{p.approvals}</td>
                  <td>
                    {p.merge_allowed ? <span className="ok">allowed</span> : <span className="muted">{p.merge_reason}</span>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Box>

      {(report.by_actor.length > 0 || report.by_kind.length > 0) && (
        <div className="row gap">
          <Box title="By actor" className="grow">
            <table className="kv compact">
              <tbody>
                {report.by_actor.map(([a, n]) => (
                  <tr key={a}>
                    <th>{a}</th>
                    <td>{n}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Box>
          <Box title="By kind" className="grow">
            <table className="kv compact">
              <tbody>
                {report.by_kind.map(([k, n]) => (
                  <tr key={k}>
                    <th>{k}</th>
                    <td>{n}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Box>
        </div>
      )}
    </>
  );
}
