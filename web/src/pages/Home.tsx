import { api } from "../api";
import { useData } from "../data";
import { Box } from "../components/Layout";
import { CodeSample } from "../components/CopyButton";
import { ModelLanding } from "../components/ModelLanding";
import { useI18n } from "../i18n";

/**
 * The site root "/" (issue #55): the D1 model landing — “No database. Only
 * rules.” The model is taught in static, marked-illustrative form here (the
 * root has no repository context); when the host has repositories, the
 * landing links into the first one's live-data collab guide (#41). The
 * host-wide repository index is the top-nav「仓库」entry at /repos (issue
 * #59) — the landing does not repeat it. An empty host shows the install
 * slate instead — there is nothing to explain yet.
 */
export function Home() {
  const owners = useData("owners", api.owners);
  const firstOwner = owners[0] ?? null;
  const repos = useData(firstOwner ? `repos:${firstOwner}` : "repos:none", () =>
    firstOwner ? api.repos(firstOwner) : Promise.resolve([] as string[]),
  );
  const live = firstOwner && repos.length > 0 ? { owner: firstOwner, repo: repos[0]! } : null;
  if (owners.length === 0) {
    return <BlankSlate />;
  }
  return <ModelLanding live={live} />;
}

/** First repo: install.sh (this host:port) sets helper + proactiveAuth + origin. */
function BlankSlate() {
  const { t } = useI18n();
  const origin = window.location.origin;
  const host = window.location.host;
  const install = `sh -c "$(curl -fsSLk '${origin}/services/public/install.sh')" -- area/repository`;
  return (
    <div className="blankslate">
      <h1>{t("blank.title")}</h1>
      <p>
        {t("blank.intro.pre")}
        <code>area/repository</code>
        {t("blank.intro.mid1")}
        <code>http.https://{host}/.proactiveAuth=auto</code>
        {t("blank.intro.mid2")}
        <code>origin</code>
        {t("blank.intro.mid3")}
        <code>{origin}/area/repository.git</code>
        {t("blank.intro.mid4")}
        <code>area/repository.git</code>
        {t("blank.intro.post")}
      </p>
      <Box title={t("blank.once")}>
        <CodeSample code={`${install}\ngit push -u origin HEAD`} />
      </Box>
      <p className="muted small">
        <code>area</code>
        {t("blank.names.and")}
        <code>repository</code>
        {t("blank.names.rule.pre")}
        <code>[A-Za-z0-9._-]</code>
        {t("blank.names.rule.post")}
      </p>
    </div>
  );
}
