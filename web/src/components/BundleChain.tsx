import type { BundleInfo } from "../api";
import { useI18n } from "../i18n";

const fmtBytes = (n: number) => {
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
const byToken = (a: BundleInfo, b: BundleInfo) => a.creation_token - b.creation_token;
const slotTime = (s: number) => new Date(s * 1000).toISOString().slice(0, 16).replace("T", " ") + "Z";

/**
 * The published bundles as the chain git walks: every full is a root; an incremental hangs under
 * the bundle whose tips are its prerequisites (`base_id`) — the previous link of its own strategy
 * when chained, else its base strategy's newest. git's creationToken heuristic downloads from the
 * newest token down until a bundle whose prerequisites it already has, so a fresh clone is the
 * whole tree and a catch-up is the links above what the client has.
 */
export function BundleChain({ bundles }: { bundles: BundleInfo[] }) {
  const { t } = useI18n();
  const listed = bundles.filter((b) => b.strategy);
  if (listed.length === 0) return <div className="muted pad">{t("bchain.none")}</div>;
  const byId = new Map(listed.map((b) => [b.sha, b]));
  const children = new Map<string, BundleInfo[]>();
  const roots: BundleInfo[] = [];
  for (const b of listed) {
    if (b.base_id && byId.has(b.base_id)) {
      const c = children.get(b.base_id) ?? [];
      c.push(b);
      children.set(b.base_id, c);
    } else {
      roots.push(b);
    }
  }
  const sortedRoots = roots.toSorted(byToken);
  for (const [k, c] of children) children.set(k, c.toSorted(byToken));
  const families = new Set(listed.map((b) => b.filter || ""));
  const total = listed.reduce((n, b) => n + b.size, 0);

  const render = (b: BundleInfo, depth: number, last: boolean): React.ReactNode => {
    const kids = children.get(b.sha) ?? [];
    const isRoot = depth === 0;
    return (
      <div key={b.sha} className="chain-node" style={{ marginLeft: depth * 22 }}>
        <div className={`chain-row ${isRoot ? "chain-root" : ""}`}>
          <span className="chain-edge" aria-hidden="true">
            {isRoot ? "●" : last ? "└─" : "├─"}
          </span>
          <span className={`pill ${b.kind}`}>{b.strategy}</span> <code>{slotTime(b.creation_token)}</code>
          <span className="muted small">
            {" "}
            · {fmtBytes(b.size)} · seq {b.at_seq}
            {b.filter ? ` · ${b.filter}` : ""}
            {isRoot ? t("bchain.noPrereq") : t("bchain.prereq", { strategy: byId.get(b.base_id)?.strategy ?? "" })}
          </span>{" "}
          <a className="small" href={b.uri} title={b.tips.map(([n, o]) => `${n} ${o.slice(0, 12)}`).join("\n")}>
            {t("bchain.download")}
          </a>
        </div>
        {kids.map((k, i) => render(k, depth + 1, i === kids.length - 1))}
      </div>
    );
  };

  return (
    <div className="pad chain">
      <div className="small muted" style={{ marginBottom: 6 }}>
        {t("bchain.summary", {
          n: listed.length,
          plural: listed.length === 1 ? "" : "s",
          bytes: fmtBytes(total),
          families:
            families.size > 1
              ? t("bchain.families", { n: families.size, filters: [...families].filter(Boolean).join(", ") })
              : "",
        })}
      </div>
      {sortedRoots.map((r) => render(r, 0, true))}
      {listed.some((b) => b.base_id && !byId.has(b.base_id)) && (
        <div className="small warn" style={{ marginTop: 6 }}>
          {t("bchain.dangling")}
        </div>
      )}
    </div>
  );
}
