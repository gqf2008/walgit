import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, type Overview, type PolicyDryRun, type PolicyValidation, type SettingsDescribe, type SettingsHistory, type SettingsValidation } from "../api";
import { invalidate, useData } from "../data";
import { Box } from "../components/Layout";
import { Maintainers } from "../components/Maintainers";
import { Link } from "react-router-dom";
import { useRepo } from "./RepoLayout";
import { useI18n } from "../i18n";

const fmtBytes = (n: number, unlimited: string) => {
  if (!n) return unlimited;
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1)} ${u[i]}`;
};
const fmtTime = (s?: string | null) => (s && !s.startsWith("1970") ? new Date(s).toLocaleString() : "—");
const show = (v: unknown): string => (v === null || v === undefined ? "—" : typeof v === "string" ? v : JSON.stringify(v));

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

/** Settings tab: scheduled tasks + placement + live plan; push policy; effective config + history. */
export function SettingsPage() {
  const { t } = useI18n();
  const { full } = useRepo();
  const d: SettingsDescribe = useData(`settings:${full}`, () => api.settings(full).describe(), 2_000);
  const o: Overview = useData(`overview:${full}`, () => api.overview(full), 2_000);
  const [section, setSection] = useState<"tasks" | "policy" | "config">("tasks");
  return (
    <div className="settings">
      <nav className="subtabs" aria-label={t("settings.nav.aria")}>
        {(
          [
            ["tasks", t("settings.tab.tasks")],
            ["policy", t("settings.tab.policy")],
            ["config", t("settings.tab.config")],
          ] as const
        ).map(([k, label]) => (
          <button key={k} type="button" className={section === k ? "subtab active" : "subtab"} onClick={() => setSection(k)}>
            {label}
          </button>
        ))}
      </nav>
      {section === "tasks" && <Tasks d={d} o={o} full={full} />}
      {section === "policy" && <PolicyEditor full={full} />}
      {section === "config" && <EffectiveConfig d={d} full={full} />}
    </div>
  );
}

// ---- 1. scheduled tasks ---------------------------------------------------------

function Tasks({ d, o, full }: { d: SettingsDescribe; o: Overview; full: string }) {
  const { t } = useI18n();
  const fb = (n: number) => fmtBytes(n, t("settings.unlimited"));
  const host = d.maintenance.this_host;
  return (
    <>
      <Box title={t("settings.strategies.title", { n: d.strategies.length, state: t(d.bundles.enabled ? "settings.enabled" : "settings.disabled") })}>
        {d.strategies.length === 0 ? (
          <div className="muted pad">{t("settings.strategies.empty")}</div>
        ) : (
          <div className="scroll-x">
            <table className="grid">
              <thead>
                <tr>
                  <th>{t("settings.th.name")}</th>
                  <th>{t("settings.th.kind")}</th>
                  <th>{t("settings.th.base")}</th>
                  <th>{t("settings.th.schedule")}</th>
                  <th>{t("settings.th.next")}</th>
                  <th>{t("settings.th.keep")}</th>
                  <th>{t("settings.th.backfill")}</th>
                  <th>{t("settings.th.minCommits")}</th>
                  <th>{t("settings.th.refs")}</th>
                </tr>
              </thead>
              <tbody>
                {d.strategies.map((s) => (
                  <tr key={s.name}>
                    <td>
                      <strong>{s.name}</strong>
                    </td>
                    <td>
                      <span className={`pill ${s.kind}`}>{s.kind}</span>
                    </td>
                    <td>{s.base ?? "—"}</td>
                    <td>
                      <code>{s.schedule}</code>
                      <div className="muted small">{s.schedule_human}</div>
                    </td>
                    <td>{fmtTime(s.next)}</td>
                    <td>
                      {s.kind === "full" ? (
                        s.keep
                      ) : s.chain ? (
                        <span title={t("settings.keep.chained.title")}>{t("settings.keep.chained", { base: s.base ?? "" })}</span>
                      ) : (
                        <span className="muted" title={t("settings.keep.twoNewest.title")}>
                          {t("settings.keep.twoNewest")}
                        </span>
                      )}
                    </td>
                    <td>{s.backfill_max || "∞"}</td>
                    <td>{s.kind === "full" ? <span className="muted">{t("settings.minCommits.neverGated")}</span> : s.min_commits}</td>
                    <td className="small">{s.refs.join(", ")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <div className="pad small muted">
          {t("settings.strategies.noteA")} <span className="pill too-small">too-small</span> {t("settings.strategies.noteB")}{" "}
          {t("settings.strategies.refsNote", { scope: t(d.bundles.main_only ? "settings.scope.mainOnly" : "settings.scope.all") })}
        </div>
      </Box>

      <Box title={t("settings.placement.title")}>
        <KV
          rows={[
            [t("settings.kv.checkpoints"), d.maintenance.checkpoints ? t("settings.on.everyPass", { secs: d.maintenance.interval_secs }) : t("settings.off")],
            [
              t("settings.kv.compaction"),
              d.compaction.enabled ? t("settings.on.trigger", { packs: d.compaction.trigger_packs, bytes: fb(d.compaction.trigger_bytes) }) : t("settings.off"),
            ],
            [
              t("settings.kv.thisInstance"),
              <span key="this-instance">
                <code>{host.name}</code> · {t("settings.roles")} {host.roles.join(", ")} ·{" "}
                {host.serves ? <span className="pill built">{t("settings.pill.serves")}</span> : <span className="pill">{t("settings.pill.notServed")}</span>}{" "}
                {host.maintains ? <span className="pill built">{t("settings.pill.maintains")}</span> : <span className="pill">{t("settings.pill.notMaintained")}</span>}
              </span>,
            ],
            [
              t("settings.kv.capacity"),
              t("settings.capacity.value", { disk: host.disk, packCap: fb(host.max_pack_bytes || host.cache_budget_bytes), cacheBudget: fb(host.cache_budget_bytes) }),
            ],
            [t("settings.kv.upstreamFollow"), <UpstreamFollow key="upstream-follow" u={d.upstream} />],
            [t("settings.kv.maintainers"), <Maintainers key="maintainers" list={o.bundle_plan.maintainers} orphaned={o.bundle_plan.orphaned} label={false} />],
          ]}
        />
        <div className="pad small muted">
          {t("settings.placement.noteA")}
          <code>[placement] maintain / maintain_exclude</code> {t("settings.placement.noteB")}{" "}
          <Link to={`/${full}/wal`}>{t("settings.placement.walPage")}</Link>
          {t("settings.placement.noteC")} <code>{full}</code> {t("settings.placement.noteD", { tab: t("settings.tab.config") })}
        </div>
      </Box>
    </>
  );
}

/** `[upstream] follow`: which refs follow which host, and what the last round here did (D33). */
function UpstreamFollow({ u }: { u: SettingsDescribe["upstream"] }) {
  const { t } = useI18n();
  if (!u || u.follow.length === 0) {
    return (
      <span>
        <span className="pill">{t("settings.off")}</span>{" "}
        <span className="muted small">
          {t("settings.upstream.hintA")} <code>[upstream] git</code> + <code>follow = ["refs/heads/main"]</code>{" "}
          {t("settings.upstream.hintB", { tab: t("settings.tab.config") })}
        </span>
      </span>
    );
  }
  const r = u.last_round;
  const pill = r ? (r.outcome === "in-sync" || r.outcome === "published" ? "pill built" : "pill stale") : "pill";
  return (
    <span>
      <code>{u.follow.join(", ")}</code> ← <code>{u.git}</code> · {t("settings.upstream.every", { secs: u.follow_interval_secs })}
      {u.token_env ? "" : ` · ${t("settings.upstream.noToken")}`}
      <div className="small">
        {r ? (
          <>
            <span className={pill}>{r.outcome}</span> {r.detail} · {fmtTime(r.at)}
            {r.outcome === "in-sync" && Object.entries(r.upstream).length > 0 && (
              <span className="muted"> · {Object.entries(r.upstream).map(([k, v]) => `${k} @ ${v.slice(0, 10)}`).join(", ")}</span>
            )}
          </>
        ) : (
          <span className="muted">{t("settings.upstream.noRound")}</span>
        )}
      </div>
    </span>
  );
}

function KV({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="kv">
      {rows.map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  );
}

// ---- 2. push policy ------------------------------------------------------------------

function PolicyEditor({ full }: { full: string }) {
  const { t } = useI18n();
  const saved = useData(`policy:${full}`, () => api.policy(full).get(), 10_000);
  const [text, setText] = useState(() => JSON.stringify(saved, null, 2));
  const [dirty, setDirty] = useState(false);
  const debounced = useDebounced(text, 400);
  const [validation, setValidation] = useState<PolicyValidation | null>(null);
  const [dry, setDry] = useState<PolicyDryRun | null>(null);
  const [busy, setBusy] = useState<"" | "dry" | "save">("");
  const [err, setErr] = useState("");
  const [last, setLast] = useState(20);
  const seq = useRef(0);

  useEffect(() => {
    if (!dirty) return;
    const n = ++seq.current;
    api
      .policy(full)
      .validate(debounced)
      .then((v) => {
        if (seq.current === n) setValidation(v);
      })
      .catch((e: Error) => {
        if (seq.current === n) setValidation({ ok: false, errors: [e.message] });
      });
  }, [debounced, dirty, full]);

  const dryRun = async () => {
    setBusy("dry");
    setErr("");
    try {
      setDry(await api.policy(full).dryRun(dirty ? text : "", last));
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy("");
    }
  };
  const save = async () => {
    setBusy("save");
    setErr("");
    try {
      await api.policy(full).put(JSON.parse(text));
      setDirty(false);
      invalidate(`policy:${full}`);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy("");
    }
  };
  const valid = !dirty || (validation?.ok ?? false);
  return (
    <>
      <Box title={t("settings.policy.title")}>
        <div className="editor">
          <textarea
            className="code-input"
            spellCheck={false}
            rows={Math.min(30, Math.max(10, text.split("\n").length + 1))}
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setDirty(true);
            }}
            aria-label="policy.json"
          />
          <div className="editor-status" aria-live="polite">
            {!dirty && (
              <span className="muted">
                {t("settings.policy.saved")}
                {Object.keys(saved).length === 0 ? t("settings.policy.savedEmpty") : ""}
              </span>
            )}
            {dirty && validation === null && <span className="muted">{t("settings.validating")}</span>}
            {dirty && validation?.ok && (
              <span className="ok">
                {t("settings.policy.valid", { rules: validation.rules ?? 0, groups: validation.groups ?? 0 })}
                {validation.protect ? t("settings.policy.validProtect") : ""}
              </span>
            )}
            {dirty && validation && !validation.ok && (
              <ul className="errors">
                {validation.errors.map((e) => (
                  <li key={e}>{e}</li>
                ))}
              </ul>
            )}
          </div>
          <div className="editor-actions">
            <label className="small">
              {t("settings.policy.dryRunLabelA")}{" "}
              <input type="number" min={1} max={200} value={last} onChange={(e) => setLast(Number(e.target.value) || 20)} className="num" />{" "}
              {t("settings.policy.dryRunLabelB")}
            </label>
            <button type="button" className="btn small" disabled={busy !== "" || !valid} onClick={dryRun}>
              {busy === "dry" ? t("settings.policy.running") : t("settings.policy.dryRun")}
            </button>
            <button type="button" className="btn small primary" disabled={busy !== "" || !dirty || !valid} onClick={save}>
              {busy === "save" ? t("settings.policy.saving") : t("settings.policy.save")}
            </button>
            <button
              type="button"
              className="btn small"
              disabled={!dirty}
              onClick={() => {
                setText(JSON.stringify(saved, null, 2));
                setDirty(false);
                setValidation(null);
              }}
            >
              {t("settings.discard")}
            </button>
          </div>
          {err && (
            <div className="flash error" role="alert">
              {err}
            </div>
          )}
        </div>
      </Box>
      {dry && (
        <Box title={t("settings.policy.dryResult.title", { pushes: dry.pushes, allowed: dry.allowed, denied: dry.denied })}>
          {dry.results.length === 0 ? (
            <div className="muted pad">{t("settings.policy.dryResult.empty")}</div>
          ) : (
            <table className="grid">
              <thead>
                <tr>
                  <th>{t("settings.th.seq")}</th>
                  <th>{t("settings.th.when")}</th>
                  <th>{t("settings.th.who")}</th>
                  <th>{t("settings.th.refs")}</th>
                </tr>
              </thead>
              <tbody>
                {dry.results.map((r) => (
                  <tr key={r.seq}>
                    <td>{r.seq}</td>
                    <td>{fmtTime(r.at)}</td>
                    <td>{r.principal}</td>
                    <td>
                      {r.refs.map((x) => (
                        <div key={x.name} className="small">
                          <span className={x.ok ? "pill built" : "pill missing"}>{x.ok ? "ok" : "ng"}</span> <code>{x.name}</code>
                          {x.force && <span className="pill">force</span>} {x.reason && <span className="muted">— {x.reason}</span>}
                        </div>
                      ))}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Box>
      )}
    </>
  );
}

// ---- 3. effective config + history -----------------------------------------------

function EffectiveConfig({ d, full }: { d: SettingsDescribe; full: string }) {
  const { t } = useI18n();
  const history: SettingsHistory = useData(`settings-history:${full}`, () => api.settings(full).history(), 5_000);
  const [text, setText] = useState(d.settings.toml);
  const [dirty, setDirty] = useState(false);
  const [message, setMessage] = useState("");
  const debounced = useDebounced(text, 400);
  const [validation, setValidation] = useState<SettingsValidation | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [filter, setFilter] = useState("");
  const seq = useRef(0);

  useEffect(() => {
    if (!dirty) return;
    const n = ++seq.current;
    api
      .settings(full)
      .validate(debounced)
      .then((v) => {
        if (seq.current === n) setValidation(v);
      })
      .catch((e: Error) => {
        if (seq.current === n) setValidation({ ok: false, errors: [e.message] });
      });
  }, [debounced, dirty, full]);

  // Fields shown: the preview when the draft validates, else the live describe.
  const view: SettingsDescribe = dirty && validation?.ok ? validation : d;
  const fields = useMemo(() => view.fields.filter((f) => !filter || f.key.includes(filter)), [view, filter]);
  const publish = async (toml: string, msg: string) => {
    setBusy(true);
    setErr("");
    try {
      await api.settings(full).put(toml, msg);
      setDirty(false);
      setMessage("");
      invalidate(`settings:${full}`);
      invalidate(`settings-history:${full}`);
      invalidate(`overview:${full}`);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const rev = d.settings.revision;
  const valid = !dirty || (validation?.ok ?? false);
  return (
    <>
      <Box title={rev ? t("settings.config.title.rev", { rev, author: "author" in d.settings ? d.settings.author : "" }) : t("settings.config.title.none")}>
        <div className="editor">
          <textarea
            className="code-input"
            spellCheck={false}
            rows={Math.min(24, Math.max(8, text.split("\n").length + 1))}
            value={text}
            placeholder={t("settings.config.placeholder")}
            onChange={(e) => {
              setText(e.target.value);
              setDirty(true);
            }}
            aria-label={t("settings.config.aria")}
          />
          <div className="editor-status" aria-live="polite">
            {!dirty && rev > 0 && "message" in d.settings && d.settings.message && <span className="muted">“{d.settings.message}”</span>}
            {dirty && validation === null && <span className="muted">{t("settings.validating")}</span>}
            {dirty && validation?.ok && <span className="ok">{t("settings.config.validPreview")}</span>}
            {dirty && validation && !validation.ok && (
              <ul className="errors">
                {validation.errors.map((e) => (
                  <li key={e}>{e}</li>
                ))}
              </ul>
            )}
          </div>
          <div className="editor-actions">
            <input className="text" placeholder={t("settings.config.messagePh")} value={message} onChange={(e) => setMessage(e.target.value)} />
            <button type="button" className="btn small primary" disabled={busy || !dirty || !valid} onClick={() => publish(text, message)}>
              {busy ? t("settings.config.publishing") : t("settings.config.publish", { rev: rev + 1 })}
            </button>
            <button
              type="button"
              className="btn small"
              disabled={!dirty}
              onClick={() => {
                setText(d.settings.toml);
                setDirty(false);
                setValidation(null);
              }}
            >
              {t("settings.discard")}
            </button>
            {rev > 0 && (
              <button type="button" className="btn small danger" disabled={busy} onClick={() => publish("", "clear")}>
                {t("settings.config.clear")}
              </button>
            )}
          </div>
          {err && (
            <div className="flash error" role="alert">
              {err}
            </div>
          )}
        </div>
      </Box>

      <Box
        title={
          <>
            {t("settings.config.effectiveTitle", { n: fields.length })}{" "}
            <input
              className="text small"
              placeholder={t("settings.config.filterPh")}
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              aria-label={t("settings.config.filterPh")}
            />
          </>
        }
      >
        <div className="scroll-x">
          <table className="grid">
            <thead>
              <tr>
                <th>{t("settings.th.key")}</th>
                <th>{t("settings.th.value")}</th>
                <th>{t("settings.th.source")}</th>
              </tr>
            </thead>
            <tbody>
              {fields.map((f) => (
                <tr key={f.key} className={f.source === "setting" ? "setting-row" : ""}>
                  <td>
                    <code>{f.key}</code>
                  </td>
                  <td className="small">
                    <code>{show(f.value)}</code>
                    {f.source === "setting" && f.host_value !== undefined && f.host_value !== null && show(f.host_value) !== show(f.value) && (
                      <span className="muted"> {t("settings.config.hostValue", { value: show(f.host_value) })}</span>
                    )}
                  </td>
                  <td>
                    {f.source === "setting" ? (
                      <span className="pill built" title={`${t("settings.config.source.repo.title")}${rev ? ` @rev ${dirty ? rev + 1 : rev}` : ""}`}>
                        {t("settings.config.source.repo")}
                        {rev ? ` @${dirty && validation?.ok ? rev + 1 : rev}` : ""}
                        {"author" in d.settings && d.settings.author && !dirty ? ` · ${d.settings.author}` : ""}
                      </span>
                    ) : (
                      <span className="pill" title={t("settings.config.source.host.title")}>
                        {t("settings.config.source.host")}
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Box>

      <Box title={t("settings.history.title", { n: history.entries.length })}>
        {history.entries.length === 0 ? (
          <div className="muted pad">{t("settings.history.empty", { seq: history.min_seq })}</div>
        ) : (
          <ol className="history">
            {history.entries.toReversed().map((e, i, arr) => {
              const prev = arr[i + 1]?.toml ?? "";
              return (
                <li key={e.seq}>
                  <div className="history-head">
                    <strong>{t("settings.history.revision", { rev: e.revision })}</strong>{" "}
                    <span className="muted">
                      · seq {e.seq} · {fmtTime(e.at)} · {e.author}
                    </span>
                    {e.message && <span> — {e.message}</span>}
                    {e.revision !== rev && (
                      <button
                        type="button"
                        className="btn small"
                        disabled={busy}
                        onClick={() => publish(e.toml, `revert to revision ${e.revision}`)}
                        title={t("settings.history.revert.title")}
                      >
                        {t("settings.history.revert")}
                      </button>
                    )}
                  </div>
                  <Diff before={prev} after={e.toml} />
                </li>
              );
            })}
          </ol>
        )}
      </Box>
    </>
  );
}

/** Minimal line diff (LCS-free: mark lines removed/added by set difference, in order). */
function Diff({ before, after }: { before: string; after: string }) {
  const { t } = useI18n();
  const a = before.split("\n").filter((l) => l.length);
  const b = after.split("\n").filter((l) => l.length);
  const aSet = new Set(a);
  const bSet = new Set(b);
  const lines: { t: "-" | "+" | " "; s: string }[] = [];
  for (const l of a) if (!bSet.has(l)) lines.push({ t: "-", s: l });
  for (const l of b) lines.push(aSet.has(l) ? { t: " ", s: l } : { t: "+", s: l });
  if (lines.length === 0) return <pre className="diff muted">{t("settings.history.emptyDoc")}</pre>;
  return (
    <pre className="diff">
      {lines.map((l, i) => (
        <div key={i} className={l.t === "+" ? "add" : l.t === "-" ? "del" : ""}>
          {l.t} {l.s}
        </div>
      ))}
    </pre>
  );
}
