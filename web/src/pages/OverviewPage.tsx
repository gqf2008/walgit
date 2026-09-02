import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { api, runOp, type OpEvent, type OpRecord, type Overview } from "../api";
import { invalidate, useData } from "../data";
import { Box } from "../components/Layout";
import { BundlePlan } from "../components/BundlePlan";
import { BundleChain } from "../components/BundleChain";
import { useRepo } from "./RepoLayout";
import { useI18n, type I18nKey } from "../i18n";

const fmtBytes = (n: number) => {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i ? 1 : 0)} ${u[i]}`;
};
const fmtTime = (s?: string) => (s && !s.startsWith("1970") ? new Date(s).toLocaleString() : "—");
const short = (s: string) => s.slice(0, 12);

function KV({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <table className="kv">
      <tbody>
        {rows.map(([k, v]) => (
          <tr key={k}>
            <th>{k}</th>
            <td>{v}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function OverviewPage() {
  const { t } = useI18n();
  const { full } = useRepo();
  // Short TTL: the WAL page is operational; an op that finished invalidates it
  // explicitly (`refresh`), otherwise it revalidates in the background.
  const o: Overview = useData(`overview:${full}`, () => api.overview(full), 2_000);
  const refresh = useCallback(() => invalidate(`overview:${full}`), [full]);
  const m = o.manifest;
  return (
    <div className="overview">
      <Box
        title={
          <>
            {t("ov.health")} <span className={`pill health-${o.health.status}`}>{o.health.status}</span>
          </>
        }
      >
        <div className="pad">
          {o.health.issues.length === 0 ? (
            <div className="muted">{t("ov.health.ok", { host: o.hostname })}</div>
          ) : (
            <ul className="issues">
              {o.health.issues.map((i) => (
                <li key={i}>{i}</li>
              ))}
            </ul>
          )}
          <div className="muted small">{t("ov.health.audit", { deep: o.health.deep })}</div>
          {o.health.suggestions.length > 0 && (
            <div className="suggestions">
              <div className="small muted" style={{ marginTop: 8 }}>
                {t("ov.health.missing")}
              </div>
              <ul className="issues">
                {o.health.suggestions.map((s) => (
                  <li key={`${s.op}:${s.params ?? ""}`}>
                    {s.reason}
                    {s.auto ? (
                      <span className="muted small">{t("ov.health.automatic", { auto: s.auto })}</span>
                    ) : (
                      <span className="pill stale small" style={{ marginLeft: 6 }}>
                        {t("ov.health.needsHuman")}
                      </span>
                    )}{" "}
                    <a href="#ops" onClick={() => window.dispatchEvent(new CustomEvent("walgit:op", { detail: s }))}>
                      {t("ov.health.run", { op: s.op, params: s.params ? ` (${s.params})` : "" })}
                    </a>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      </Box>

      <OpsBox repo={full} overview={o} onChanged={refresh} />

      <div className="row gap">
        <Box title={t("ov.manifest")} className="grow">
          <KV
            rows={[
              // oxlint-disable-next-line react/jsx-key -- label/value tuples, not a rendered list
              [t("ov.kv.version"), <code>{m.version}</code>],
              [t("ov.kv.nextSeq"), m.next_seq],
              [t("ov.kv.minSeq"), m.min_seq],
              [t("ov.kv.entries"), m.entries],
              [t("ov.kv.sealedSegments"), m.segments.length],
              [t("ov.kv.inlineTail"), m.tail_entries],
              [t("ov.kv.lastPush"), fmtTime(m.last_push)],
              // oxlint-disable-next-line react/jsx-key
              [t("ov.kv.advertisedBundle"), m.advertised_bundle_uri ? <code className="wrap">{m.advertised_bundle_uri}</code> : "—"],
            ]}
          />
        </Box>
        <Box title={t("ov.local", { host: o.hostname })} className="grow">
          <KV
            rows={[
              [
                t("ov.kv.instance"),
                <span key="instance">
                  {o.instance.kind === "ssd" ? t("ov.instance.ssd") : o.instance.kind === "serverless" ? t("ov.instance.serverless") : o.instance.kind} ·{" "}
                  {o.instance.shape} · <code>{o.instance.revision || o.instance.name}</code>
                </span>,
              ],
              [t("ov.kv.build"), <code key="build">{o.instance.version}</code>],
              // oxlint-disable-next-line react/jsx-key
              [t("ov.kv.version"), <code>{o.local.version || "—"}</code>],
              [t("ov.kv.nextSeq"), o.local.next_seq],
              [t("ov.kv.bootstrapSeq"), o.local.bootstrap],
              [t("ov.kv.reconciled"), o.local.reconciled ? t("ov.yes") : t("ov.no")],
              [t("ov.kv.sizeOnDisk"), fmtBytes(o.local.size_bytes)],
            ]}
          />
        </Box>
      </div>

      <div className="row gap">
        <Box title={t("ov.packs")} className="grow">
          <KV
            rows={[
              [t("ov.kv.livePacks"), o.packs.live],
              [t("ov.kv.liveBytes"), fmtBytes(o.packs.live_bytes)],
              [t("ov.kv.pushPacks"), o.packs.pushes],
              [t("ov.kv.compactions"), o.compactions.length],
            ]}
          />
        </Box>
        <Box title={t("ov.checkpoints")} className="grow">
          <KV
            rows={[
              [
                t("ov.kv.packset"),
                m.packset
                  ? t("ov.checkpoint.packset", {
                      seq: m.packset.at_seq,
                      packs: m.packset.packs,
                      bytes: fmtBytes(m.packset.bytes),
                      time: fmtTime(m.packset.created),
                      creator: m.packset.creator,
                    })
                  : "—",
              ],
              [
                t("ov.kv.bundle"),
                m.checkpoint ? (
                  <>
                    {t("ov.checkpoint.bundle", { seq: m.checkpoint.at_seq, bytes: fmtBytes(m.checkpoint.size) })} <code>{short(m.checkpoint.sha)}</code>
                  </>
                ) : (
                  "—"
                ),
              ],
            ]}
          />
        </Box>
      </div>

      <Box title={t("ov.bundle.listed", { n: o.bundles.length })}>
        <BundleChain bundles={o.bundles} />
      </Box>

      <Box title={t("ov.bundle.slots")}>
        <BundlePlan plan={o.bundle_plan} />
      </Box>

      <Box title={t("ov.compactions", { n: o.compactions.length })}>
        {o.compactions.length === 0 ? (
          <div className="muted pad">{t("ov.compactions.none")}</div>
        ) : (
          <table className="grid">
            <thead>
              <tr>
                <th>{t("ov.th.seq")}</th>
                <th>{t("ov.th.level")}</th>
                <th>{t("ov.th.folds")}</th>
                <th>{t("ov.th.pack")}</th>
                <th>{t("ov.th.superseded")}</th>
                <th>{t("ov.th.at")}</th>
                <th>{t("ov.th.primary")}</th>
              </tr>
            </thead>
            <tbody>
              {o.compactions.toReversed().map((c) => (
                <tr key={c.seq}>
                  <td>{c.seq}</td>
                  <td>{c.level}</td>
                  <td>
                    {c.first_seq}..{c.last_seq}
                  </td>
                  <td>{fmtBytes(c.pack_size)}</td>
                  <td>{t("ov.compactions.superseded", { packs: c.superseded_packs, bytes: fmtBytes(c.superseded_bytes) })}</td>
                  <td>{fmtTime(c.at)}</td>
                  <td>{c.primary}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Box>

      <Segments segments={m.segments} />
    </div>
  );
}

// ---- Maintenance ops ---------------------------------------------------------

const PARAM_KEYS: Record<string, I18nKey> = {
  connectivity: "ov.param.connectivity",
  force: "ov.param.force",
  base: "ov.param.base",
};

function OpsBox({ repo, overview, onChanged }: { repo: string; overview: Overview; onChanged: () => void }) {
  const { t } = useI18n();
  const specs = overview.ops.available;
  const [running, setRunning] = useState<string | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [status, setStatus] = useState<{ ok?: boolean; text: string } | null>(null);
  const [flags, setFlags] = useState<Record<string, boolean>>({});
  const [strategy, setStrategy] = useState<string>("");
  const abort = useRef<AbortController | null>(null);
  const logRef = useRef<HTMLPreElement>(null);

  const logLen = log.length;
  useEffect(() => {
    if (logLen) logRef.current?.scrollTo({ top: logRef.current.scrollHeight });
  }, [logLen]);

  const run = useCallback(
    async (op: string, params: Record<string, string>) => {
      // One op at a time per page: `abort.current` is set while one runs.
      if (abort.current && !abort.current.signal.aborted) return;
      setRunning(op);
      setLog([]);
      setStatus({ text: t("ov.ops.starting", { op }) });
      abort.current = new AbortController();
      const t0 = performance.now();
      try {
        await runOp(
          repo,
          op,
          params,
          (ev: OpEvent) => {
            if (ev.event === "log") setLog((l) => [...l, ev.line]);
            else if (ev.event === "started") setStatus({ text: t("ov.ops.runningOn", { op, host: ev.record.hostname }) });
            else if (ev.event === "done") setStatus({ ok: true, text: ev.record.summary });
            else if (ev.event === "error") setStatus({ ok: false, text: ev.message });
          },
          abort.current.signal,
        );
      } catch (e) {
        setStatus({ ok: false, text: (e as Error).message });
      } finally {
        setLog((l) => [...l, t("ov.ops.finished", { s: ((performance.now() - t0) / 1000).toFixed(1) })]);
        abort.current = null;
        setRunning(null);
        onChanged();
      }
    },
    [repo, onChanged, t], // oxlint-disable-line react/memo-dependencies -- refs are stable
  );

  // Health suggestions dispatch "walgit:op" with {op, params}.
  useEffect(() => {
    const h = (e: Event) => {
      const d = (e as CustomEvent<{ op: string; params?: string }>).detail;
      const params: Record<string, string> = {};
      if (d.params) for (const [k, v] of new URLSearchParams(d.params)) params[k] = v;
      void run(d.op, params);
    };
    window.addEventListener("walgit:op", h);
    return () => window.removeEventListener("walgit:op", h);
  }, [run]);

  const paramsFor = (op: string, spec: { params: string[] }) => {
    const p: Record<string, string> = {};
    for (const k of spec.params) if (k !== "strategy" && flags[`${op}.${k}`]) p[k] = "1";
    if (op === "bundle" && strategy) p.strategy = strategy;
    return p;
  };

  return (
    <Box title={t("ov.maintenance")} id="ops">
      <div className="pad">
        <p className="small muted">{t("ov.ops.explainer", { host: overview.hostname })}</p>
        <table className="kv ops-table">
          <tbody>
            {specs.map((s) => (
              <tr key={s.id}>
                <th>
                  <button className="btn small" disabled={!!running} onClick={() => void run(s.id, paramsFor(s.id, s))}>
                    {running === s.id ? t("ov.ops.running") : s.label}
                  </button>
                </th>
                <td>
                  <div>{s.description}</div>
                  <div className="op-params">
                    {s.params.flatMap((k) => {
                      if (k === "strategy") return [];
                      const help = PARAM_KEYS[k];
                      return (
                        <label key={k} className="small muted">
                          <input
                            type="checkbox"
                            checked={!!flags[`${s.id}.${k}`]}
                            onChange={(e) => setFlags({ ...flags, [`${s.id}.${k}`]: e.target.checked })}
                          />{" "}
                          {help ? t(help) : k}
                        </label>
                      );
                    })}
                    {s.params.includes("strategy") && (
                      <label className="small muted">
                        {t("ov.ops.strategy")}{" "}
                        <select value={strategy} onChange={(e) => setStrategy(e.target.value)}>
                          <option value="">{t("ov.ops.strategy.default")}</option>
                          <option value="due">{t("ov.ops.strategy.due")}</option>
                          {overview.ops.bundle_strategies.map((b) => (
                            <option key={b} value={b}>
                              {b}
                            </option>
                          ))}
                        </select>
                      </label>
                    )}
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {(status || log.length > 0) && (
          <div className="op-output">
            {status && (
              <div className={`op-status ${status.ok === undefined ? "" : status.ok ? "health-ok" : "health-error"}`}>
                {status.text}
              </div>
            )}
            <pre className="code-block op-log" ref={logRef}>
              {log.join("\n")}
            </pre>
          </div>
        )}
      </div>
      {overview.ops.recent.length > 0 && <OpLog recent={overview.ops.recent} />}
    </Box>
  );
}


// ---- Op log ------------------------------------------------------------------

/** The same op with the same outcome shape (summary minus digits) in a row: one line, a count. */
function shape(r: OpRecord): string {
  return `${r.kind}|${r.ok === undefined ? "running" : r.ok ? "ok" : "failed"}|${r.summary.replace(/[0-9a-f]{7,}|\d+/g, "#")}`;
}

function OpLog({ recent }: { recent: OpRecord[] }) {
  const { t } = useI18n();
  const groups: { first: OpRecord; last: OpRecord; n: number; hosts: Set<string> }[] = [];
  for (const r of recent) {
    const g = groups.at(-1);
    if (g && shape(g.first) === shape(r)) {
      g.n += 1;
      g.last = r;
      g.hosts.add(r.hostname);
    } else {
      groups.push({ first: r, last: r, n: 1, hosts: new Set([r.hostname]) });
    }
  }
  return (
    <table className="grid">
      <thead>
        <tr>
          <th>{t("ov.th.op")}</th>
          <th>{t("ov.th.when")}</th>
          <th>{t("ov.th.took")}</th>
          <th>{t("ov.th.result")}</th>
          <th>{t("ov.th.instance")}</th>
        </tr>
      </thead>
      <tbody>
        {groups.map((g) => {
          const r = g.first;
          return (
            <tr key={r.id} className={g.n > 1 ? "muted" : ""}>
              <td>
                <code>{r.kind}</code>
                {g.n > 1 && <span className="pill small" title={t("ov.ops.consecutive", { n: g.n })}> ×{g.n}</span>}
              </td>
              <td className="small">
                {g.n > 1 ? (
                  <>
                    {fmtTime(g.last.started)} → {fmtTime(r.started)}
                  </>
                ) : (
                  fmtTime(r.started)
                )}
              </td>
              <td>{r.ok === undefined ? "…" : `${(r.elapsed_ms / 1000).toFixed(1)}s`}</td>
              <td>
                <span className={`pill ${r.ok === undefined ? "" : r.ok ? "health-ok" : "health-error"}`}>
                  {r.ok === undefined ? t("ov.ops.state.running") : r.ok ? t("ov.ops.state.ok") : t("ov.ops.state.failed")}
                </span>{" "}
                {g.n > 1 ? r.summary.replace(/\d{10}/, "…") : r.summary}
              </td>
              <td className="small">{[...g.hosts].map((h) => h.slice(0, 8)).join(", ")}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}


// ---- Segments ----------------------------------------------------------------

/** One object per publish batch: the list is the WAL's length. Show its shape, not every row. */
function Segments({ segments }: { segments: Overview["manifest"]["segments"] }) {
  const { t } = useI18n();
  const [all, setAll] = useState(false);
  if (segments.length === 0) {
    return (
      <Box title={t("ov.segments", { n: 0 })}>
        <div className="muted pad">{t("ov.segments.none")}</div>
      </Box>
    );
  }
  const bytes = segments.reduce((n, x) => n + x.size, 0);
  const first = segments[0]!;
  const last = segments.at(-1)!;
  const shown = all ? segments.toReversed() : segments.slice(-5).toReversed();
  return (
    <Box title={t("ov.segments", { n: segments.length })}>
      <div className="pad small muted">
        {t("ov.segments.summary", {
          first: first.first_seq,
          last: last.last_seq,
          n: segments.length,
          plural: segments.length === 1 ? "" : "s",
          bytes: fmtBytes(bytes),
        })}{" "}
        {segments.length > 5 && (
          <button className="btn link small" onClick={() => setAll(!all)}>
            {all ? t("ov.segments.newest") : t("ov.segments.all", { n: segments.length })}
          </button>
        )}
      </div>
      <table className="grid">
        <thead>
          <tr>
            <th>{t("ov.th.key")}</th>
            <th>{t("ov.th.seqs")}</th>
            <th>{t("ov.th.size")}</th>
          </tr>
        </thead>
        <tbody>
          {shown.map((x) => (
            <tr key={x.key}>
              <td>
                <code>{x.key}</code>
              </td>
              <td>
                {x.first_seq}..{x.last_seq}
              </td>
              <td>{fmtBytes(x.size)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Box>
  );
}
