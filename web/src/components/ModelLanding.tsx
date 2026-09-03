import { Link } from "react-router-dom";
import type { ReactNode } from "react";
import { useI18n, decisionLabel, kindLabel, statusLabel, type TFunc } from "../i18n";

/**
 * Home model landing (issue #55): “No database. Only rules.” — the D1
 * collaboration layer's mental model in plain language, the first thing a
 * visitor of "/" meets. Everything here is illustrative and marked as such
 * (the page has no repository context); the same story told on a
 * repository's own live data is the collab guide page (#41), linked from
 * here when the host has a repository.
 */

const ORIGIN_POST = "https://cursor.com/blog/git-at-any-scale";
const TURBOPUFFER = "https://turbopuffer.com/blog/turbopuffer";

export function ModelLanding({ live }: { live: { owner: string; repo: string } | null }) {
  const { t, lang } = useI18n();
  const numerals = lang === "zh-CN" ? ["壹", "贰", "叁"] : lang === "zh-TW" ? ["壹", "貳", "參"] : ["1", "2", "3"];
  return (
    <div className="model-wrap">
      <header className="model-hero">
        <div className="eyebrow">{t("home.eyebrow")}</div>
        <h1 className="model-title">
          {t("home.h1.a")} <span className="model-accent">{t("home.h1.b")}</span>
        </h1>
        <p className="model-lede">{t("home.lede")}</p>
        {live && (
          <div className="model-live">
            <Link to={`/${live.owner}/${live.repo}/collab/guide`} className="btn btn-primary">
              {t("home.live")}
            </Link>
          </div>
        )}
      </header>

      {/* ① An issue is an append-only chain */}
      <section aria-labelledby="model-1">
        <ModelHead id="model-1" no={numerals[0]!} title={t("home.m1.t")} />
        <p className="model-sub">{t("home.m1.sub")}</p>
        <div className="model-chain" aria-label={t("home.m1.t")}>
          <ChainRow kind="issue" title={t("home.m1.demo.issue")} act={t("home.m1.demo.act.open")} ts={1788316320} />
          <ChainRow kind="status" title={statusLabel(t, "needs-review")} act={t("home.m1.demo.act.move")} ts={1788316321} />
          <ChainRow kind="review" title={decisionLabel(t, "approve")} act={t("home.m1.demo.act.conclude")} ts={1788316323} />
        </div>
        <div className="model-illus mono">{t("home.illus")}</div>
        <div className="model-note">{t("home.m1.note")}</div>
      </section>

      {/* ② The board exists nowhere — it is computed */}
      <section aria-labelledby="model-2">
        <ModelHead id="model-2" no={numerals[1]!} title={t("home.m2.t")} />
        <p className="model-sub">{t("home.m2.sub")}</p>
        <div className="model-proj">
          <ProjPanel
            who={<code>walgit collab board</code>}
            caption={t("home.m2.l")}
            sub={t("home.m2.l.sub")}
            cols={boardCols(t)}
          />
          <div className="model-eq">
            <span className="guide-equiv">≡ {t("guide.s2.equiv")}</span>
          </div>
          <ProjPanel
            who={<code>{"GET /<owner>/<repo>/api/collab/board"}</code>}
            caption={t("home.m2.r")}
            sub={t("home.m2.r.sub")}
            cols={boardCols(t)}
          />
        </div>
        <div className="model-refs mono">
          {t("home.m2.refs.a")}
          <b>refs/collab/*</b>
          {t("home.m2.refs.b", { n: 6 })}
          <b>.walgit/board.toml</b>
          {t("home.m2.refs.c")}
        </div>
        <div className="model-note">{t("home.m2.note")}</div>
      </section>

      {/* ③ Two runners may both run — only one counts */}
      <section aria-labelledby="model-3">
        <ModelHead id="model-3" no={numerals[2]!} title={t("home.m3.t")} />
        <p className="model-sub">{t("home.m3.sub")}</p>
        <div className="guide-steps">
          {[
            [t("guide.s3.step1"), t("guide.s3.step1.sub")],
            [t("guide.s3.step2"), t("guide.s3.step2.sub")],
            [t("guide.s3.step3"), t("guide.s3.step3.sub")],
          ].map(([head, sub], i) => (
            <div key={head} style={{ display: "contents" }}>
              {i > 0 && <div className="guide-arrow">→</div>}
              <div className="guide-step">
                <div className="strong">{head}</div>
                <div className="muted" style={{ fontSize: "0.9em" }}>{sub}</div>
              </div>
            </div>
          ))}
        </div>
        <div className="model-fx">
          <span className="fx-chip mono fx-winner">{t("home.m3.fx.winner")}</span>
          <span className="fx-chip mono">{t("home.m3.fx.any")}</span>
          <span className="fx-chip mono">{t("home.m3.fx.once")}</span>
          <span className="fx-chip mono">{t("home.m3.fx.ttl")}</span>
        </div>
        <div className="model-note">{t("home.m3.note")}</div>
      </section>

      {/* Looks like a bug — isn't */}
      <section aria-labelledby="model-bugs">
        <ModelHead id="model-bugs" no={null} title={t("home.bugs.t")} />
        <p className="model-sub">{t("home.bugs.lede")}</p>
        <div className="model-bugs">
          {(
            [
              [t("home.bug1.sight"), t("home.bug1.truth")],
              [t("home.bug2.sight"), t("home.bug2.truth")],
              [t("home.bug3.sight"), t("home.bug3.truth")],
              [t("home.bug4.sight"), t("home.bug4.truth")],
            ] as [string, string][]
          ).map(([sight, truth]) => (
            <div key={sight} className="model-bcard">
              <div className="model-sight">{sight}</div>
              <div className="model-truth">{truth}</div>
            </div>
          ))}
        </div>
      </section>

      {/* When in doubt — three commands */}
      <footer className="model-foot">
        <ModelHead no={null} title={t("home.cmds.t")} />
        <div className="model-cmds">
          {(
            [
              ["walgit collab thread <id>", t("home.cmd1.what")],
              ["walgit collab board", t("home.cmd2.what")],
              ["walgit collab report", t("home.cmd3.what")],
            ] as [string, string][]
          ).map(([cmd, what]) => (
            <div key={cmd} className="model-cmd">
              <div className="mono">{cmd}</div>
              <div className="model-cmd-what">{what}</div>
            </div>
          ))}
        </div>
        <p className="model-endnote">
          {t("home.endnote.a")}
          {t("guide.protocol")}
        </p>
        <p className="model-endnote">
          {t("home.skill.pre")}
          <a href="/SKILL.md">SKILL.md</a>
        </p>
        <p className="model-endnote">
          {t("home.origin.a")}
          <a href={ORIGIN_POST} target="_blank" rel="noreferrer">{t("home.origin.l1")}</a>
          {t("home.origin.b")}
          <a href={TURBOPUFFER} target="_blank" rel="noreferrer">{t("home.origin.l2")}</a>
          {t("home.origin.c")}
        </p>
      </footer>
    </div>
  );
}

/** Eyebrow-less section head: numeral badge (when numbered) + title. */
function ModelHead({ id, no, title }: { id?: string; no: string | null; title: string }) {
  return (
    <div className="model-head">
      {no && <span className="model-no serif">{no}</span>}
      <h2 id={id}>{title}</h2>
    </div>
  );
}

/** One appended entry in the illustrative chain: kind chip, what changed, seal. */
function ChainRow({ kind, title, act, ts }: { kind: string; title: string; act: string; ts: number }) {
  const { t } = useI18n();
  return (
    <div className="model-entry">
      <span className="model-kind mono">{kindLabel(t, kind)}</span>
      <div className="model-entry-body">
        <div className="model-entry-title">{title}</div>
        <div className="model-entry-meta">
          alice · {act} · <span className="mono">ts {ts}</span>
        </div>
      </div>
      <span className="model-seal">
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none" aria-hidden>
          <path d="M2 6.5L4.8 9L10 3" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
        </svg>
        {t("entry.verified")}
      </span>
    </div>
  );
}

/** One side of the ≡ projection: caption + command + a tiny board. */
function ProjPanel({
  who,
  caption,
  sub,
  cols,
}: {
  who: ReactNode;
  caption: string;
  sub: string;
  cols: [string, { id: string; title: string }[]][];
}) {
  return (
    <div className="model-panel">
      <h3>{caption}</h3>
      <div className="model-who">
        {who}
        <small>{sub}</small>
      </div>
      <div className="model-mini">
        {cols.map(([name, cards]) => (
          <div key={name} className="model-mini-col">
            <span className="model-col-name">{name}</span>
            {cards.map((c) => (
              <span key={c.id} className="model-card mono">{c.id} {c.title}</span>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

/** The demo board shown on both sides of the ≡: three status columns. */
function boardCols(t: TFunc): [string, { id: string; title: string }[]][] {
  return [
    [statusLabel(t, "needs-review"), [{ id: "w1", title: t("home.m1.demo.issue") }]],
    [statusLabel(t, "in-progress"), [{ id: "w2", title: t("home.m2.demo.w2") }]],
    [statusLabel(t, "open"), [{ id: "w3", title: t("home.m2.demo.w3") }]],
  ];
}
