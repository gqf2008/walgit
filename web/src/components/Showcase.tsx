import { Link } from "react-router-dom";
import { api } from "../api";
import { useData } from "../data";
import { useI18n, kindLabel, statusLabel } from "../i18n";

/**
 * Landing D1 showcase (issue #43): the 30-second version of the collab guide,
 * on the most prominent surface of the product. Three models, each with a
 * mini-visual (chain timeline / byte-identical panels / claim-race banner),
 * then a CTA into the first repository's live guide page, where the same
 * story is told with that repository's real data. All visuals are
 * illustrative (marked); nothing here is repo state.
 */
export function Showcase({ owners }: { owners: string[] }) {
  const { t } = useI18n();
  const firstOwner = owners[0] ?? null;
  const repos = useData(firstOwner ? `repos:${firstOwner}` : "repos:none", () =>
    firstOwner ? api.repos(firstOwner) : Promise.resolve([] as string[]),
  );
  const first = firstOwner && repos[0] ? `${firstOwner}/${repos[0]}` : null;
  return (
    <section className="showcase" aria-labelledby="showcase-title">
      <h2 id="showcase-title" className="showcase-title">{t("showcase.title")}</h2>
      <p className="showcase-lede">{t("showcase.lede")}</p>
      <div className="showcase-grid">
        <div className="showcase-card">
          <div className="showcase-visual">
            <div className="guide-chain">
              {(["issue", "comment", "status"] as const).map((k) => (
                <div key={k} className="guide-entry">
                  {kindLabel(t, k)}
                  <span className="ok" style={{ marginLeft: 6 }}>{t("entry.verified")}</span>
                </div>
              ))}
            </div>
            <div className="showcase-illus muted">{t("showcase.illustrative")}</div>
          </div>
          <div className="strong">{t("showcase.m1.t")}</div>
          <div className="muted">{t("showcase.m1.b")}</div>
        </div>
        <div className="showcase-card">
          <div className="showcase-visual showcase-panels">
            <MiniBoard />
            <div className="guide-equiv">≡</div>
            <MiniBoard />
          </div>
          <div className="strong">{t("showcase.m2.t")}</div>
          <div className="muted">{t("showcase.m2.b")}</div>
        </div>
        <div className="showcase-card">
          <div className="showcase-visual guide-steps">
            {[t("guide.s3.step1"), t("guide.s3.step2"), t("guide.s3.step3")].map((s, i) => (
              <div key={s} style={{ display: "contents" }}>
                {i > 0 && <div className="guide-arrow">→</div>}
                <div className="guide-step strong">{s}</div>
              </div>
            ))}
          </div>
          <div className="strong">{t("showcase.m3.t")}</div>
          <div className="muted">{t("showcase.m3.b")}</div>
        </div>
      </div>
      {first && (
        <div className="showcase-cta">
          <Link to={`/${first}/collab/guide`} className="btn btn-primary">
            {t("showcase.cta.guide")}
          </Link>
        </div>
      )}
    </section>
  );
}

/** Two identical tiny boards frame the ≡ of the deterministic projection. */
function MiniBoard() {
  const { t } = useI18n();
  const cols: [string, string[]][] = [
    [statusLabel(t, "in-progress"), ["w-1"]],
    [statusLabel(t, "needs-review"), ["w-2", "w-3"]],
  ];
  return (
    <div className="guide-mini grow">
      <div className="row gap" style={{ alignItems: "flex-start" }}>
        {cols.map(([name, cards]) => (
          <div key={name} className="guide-mini-col grow">
            <div className="muted">{name}</div>
            {cards.map((c) => (
              <div key={c} className="guide-mini-card mono">{c}</div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
