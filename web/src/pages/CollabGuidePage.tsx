import { Link } from "react-router-dom";
import { api, type CollabBoard, type CollabEntryRef, type CollabThread } from "../api";
import { useRepo } from "./RepoLayout";
import { useData } from "../data";
import { Box } from "../components/Layout";
import { useI18n, kindLabel, type TFunc } from "../i18n";

/**
 * 「了解 D1 协作」guide page (issue #41): the collaboration layer's mental
 * model taught inside the product, with this repository's own live data —
 * a real entry chain, the real board rendered twice to demonstrate the
 * byte-identical projection, the claim-race convergence rule, and the four
 * "looks like a bug — isn't" sights. All prose goes through i18n; data
 * values (kinds, statuses) are translated at render time like everywhere
 * else.
 */

function fmtTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString();
}

export function CollabGuidePage() {
  const { t } = useI18n();
  const { full } = useRepo();
  const data = useData(`collab:${full}:guide`, async () => {
    const report = await api.collab(full).report();
    const board = await api.collab(full).board();
    const featuredId = report.threads.find((th) => th.entries >= 2)?.id ?? null;
    const thread = featuredId ? await api.collab(full).thread(featuredId) : null;
    return { report, board, thread };
  });
  return (
    <>
      <div className="pad">
        <Link to={`/${full}/collab`} className="muted">{t("back.collab")}</Link>
      </div>
      <Box title={t("guide.title")}>
        <div className="pad">{t("guide.lede")}</div>
      </Box>
      <ChainSection full={full} thread={data.thread} />
      <ProjectionSection board={data.board} />
      <ClaimSection />
      <BugsSection />
      <CommandsSection />
    </>
  );
}

/** ① An issue is an append-only chain — shown on the repo's real thread. */
function ChainSection({ full, thread }: { full: string; thread: CollabThread | null }) {
  const { t } = useI18n();
  return (
    <Box title={t("guide.s1.title")}>
      <div className="pad">{t("guide.s1.body")}</div>
      <div className="pad">
        {thread ? (
          <>
            <div className="muted" style={{ marginBottom: 8 }}>
              {t("guide.s1.live")}{" "}
              <Link to={`/${full}/collab/thread/${encodeURIComponent(thread.id)}`}>{t("guide.openThread")}</Link>
            </div>
            <div className="guide-chain">
              {thread.entries.slice(0, 6).map((e) => (
                <ChainEntry key={e.oid} e={e} t={t} />
              ))}
            </div>
          </>
        ) : (
          <>
            <div className="muted" style={{ marginBottom: 8 }}>{t("guide.s1.sample")}</div>
            <div className="guide-chain">
              {(["issue", "comment", "status"] as const).map((k) => (
                <div key={k} className="guide-entry muted">{kindLabel(t, k)}</div>
              ))}
            </div>
          </>
        )}
      </div>
      <div className="pad muted">{t("guide.s1.note")}</div>
    </Box>
  );
}

function ChainEntry({ e, t }: { e: CollabEntryRef; t: TFunc }) {
  const body = e.entry.body as Record<string, unknown>;
  const title = e.entry.kind === "issue" ? String(body.title ?? "") : "";
  return (
    <div className="guide-entry">
      <strong>{kindLabel(t, e.entry.kind)}</strong>
      {title && <> · {title}</>} · {e.entry.actor} · {fmtTime(e.entry.ts)} ·{" "}
      {e.verified ? <span className="ok">{t("entry.verified")}</span> : <span>{t("entry.unverified")}</span>}
    </div>
  );
}

/** ② The board exists nowhere — the same live board rendered twice, ≡. */
function ProjectionSection({ board }: { board: CollabBoard }) {
  const { t } = useI18n();
  return (
    <Box title={t("guide.s2.title")}>
      <div className="pad">{t("guide.s2.body")}</div>
      <div className="pad row gap" style={{ alignItems: "stretch" }}>
        <MiniBoard board={board} caption={t("guide.s2.here")} sub={t("guide.s2.here.sub")} />
        <div className="guide-equiv">≡ {t("guide.s2.equiv")}</div>
        <MiniBoard board={board} caption={t("guide.s2.any")} sub={t("guide.s2.any.sub")} />
      </div>
      <div className="pad muted">{t("guide.s2.note")}</div>
    </Box>
  );
}

function MiniBoard({ board, caption, sub }: { board: CollabBoard; caption: string; sub: string }) {
  return (
    <div className="guide-mini grow">
      <div className="guide-mini-head">
        <strong>{caption}</strong>
        <span className="muted">{sub}</span>
      </div>
      <div className="row gap" style={{ alignItems: "flex-start" }}>
        {board.columns.slice(0, 3).map((col) => (
          <div key={col.name} className="guide-mini-col grow">
            <div className="muted">{col.name} ({col.cards.length})</div>
            {col.cards.slice(0, 2).map((c) => (
              <div key={c.id} className="guide-mini-card">{c.title || c.id}</div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

/** ③ The claim race: both may run, one deterministic winner. */
function ClaimSection() {
  const { t } = useI18n();
  const steps: [string, string][] = [
    [t("guide.s3.step1"), t("guide.s3.step1.sub")],
    [t("guide.s3.step2"), t("guide.s3.step2.sub")],
    [t("guide.s3.step3"), t("guide.s3.step3.sub")],
  ];
  return (
    <Box title={t("guide.s3.title")}>
      <div className="pad">{t("guide.s3.body")}</div>
      <div className="pad guide-steps">
        {steps.map(([head, sub], i) => (
          <div key={head} style={{ display: "contents" }}>
            {i > 0 && <div className="guide-arrow">→</div>}
            <div className="guide-step">
              <div className="strong">{head}</div>
              <div className="muted" style={{ fontSize: "0.9em" }}>{sub}</div>
            </div>
          </div>
        ))}
      </div>
      <div className="pad muted">{t("guide.s3.note")}</div>
    </Box>
  );
}

/** ④ The four "looks like a bug — isn't" sights. */
function BugsSection() {
  const { t } = useI18n();
  const bugs: [string, string][] = [
    [t("guide.bug1.sight"), t("guide.bug1.truth")],
    [t("guide.bug2.sight"), t("guide.bug2.truth")],
    [t("guide.bug3.sight"), t("guide.bug3.truth")],
    [t("guide.bug4.sight"), t("guide.bug4.truth")],
  ];
  return (
    <Box title={t("guide.bugs.title")}>
      <div className="pad">{t("guide.bugs.lede")}</div>
      <div className="pad guide-bcards">
        {bugs.map(([sight, truth]) => (
          <div key={sight} className="guide-bcard">
            <div className="strong">{sight}</div>
            <div className="muted">{truth}</div>
          </div>
        ))}
      </div>
    </Box>
  );
}

/** The three CLI commands that answer any doubt. */
function CommandsSection() {
  const { t } = useI18n();
  const cmds: [string, string][] = [
    ["walgit collab thread <id>", t("guide.cmd1.what")],
    ["walgit collab board", t("guide.cmd2.what")],
    ["walgit collab report", t("guide.cmd3.what")],
  ];
  return (
    <Box title={t("guide.cmds.title")}>
      <div className="pad">
        {cmds.map(([cmd, what]) => (
          <div key={cmd} className="guide-cmd">
            <code className="mono">{cmd}</code>
            <span className="muted">{what}</span>
          </div>
        ))}
        <div className="muted" style={{ marginTop: 8 }}>{t("guide.protocol")}</div>
      </div>
    </Box>
  );
}
