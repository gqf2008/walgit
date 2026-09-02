import { useEffect, useState } from "react";
import { useI18n, type I18nKey } from "../i18n";

/** Which instance answered — kind is the loud part (a serverless host vs the SSD host must
 * be distinguishable at a glance), then the name, revision, build. */
export interface InstanceInfo {
  kind: "serverless" | "ssd" | "dev" | string;
  name: string;
  revision: string;
  instance: string;
  version: string;
  roles: string[];
  disk: "tmpfs" | "ssd" | string;
}

const KIND_KEYS: Record<string, I18nKey> = {
  serverless: "footer.kind.serverless",
  ssd: "footer.kind.ssd",
  dev: "footer.kind.dev",
};

export function InstanceFooter() {
  const { t } = useI18n();
  const [info, setInfo] = useState<InstanceInfo | null>(null);
  useEffect(() => {
    let live = true;
    // Non-repo instance facts live at /services/api/instance (D27: /api/v1 is discovery/me/owners only).
    fetch("/services/api/instance", { headers: { Accept: "application/json" }, credentials: "same-origin" })
      .then((r) => (r.ok ? r.json() : null))
      .then((j) => live && j && setInfo(j as InstanceInfo))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  if (!info) return null;
  const where = info.kind === "serverless" ? `${info.revision || info.name}${info.instance ? ` · ${info.instance}` : ""}` : info.name;
  const kindKey = KIND_KEYS[info.kind];
  return (
    <footer
      className={`instance-footer kind-${info.kind}`}
      title={t("footer.title", { roles: info.roles.join(", "), disk: info.disk })}
    >
      <span className="instance-kind">{kindKey ? t(kindKey) : info.kind}</span>
      <span className="instance-where">{where}</span>
      <span className="instance-version">{info.version}</span>
    </footer>
  );
}
