import { Link } from "react-router-dom";
import { api } from "../api";
import { useData } from "../data";
import { Box } from "../components/Layout";
import { Hero } from "../components/Hero";
import { Showcase } from "../components/Showcase";
import { CodeSample } from "../components/CopyButton";
import { useI18n } from "../i18n";

export function Owners() {
  const { t } = useI18n();
  const owners = useData("owners", api.owners);
  if (owners.length === 0) {
    return <BlankSlate />;
  }
  return (
    <>
      <Hero />
      <Showcase owners={owners} />
      <h2 className="page-title">{t("home.reposByOwner")}</h2>
      <Box>
        <ul className="list">
          {owners.map((o) => (
            <li key={o}>
              <Link to={`/${o}`} className="strong">
                {o}
              </Link>
            </li>
          ))}
        </ul>
      </Box>
    </>
  );
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
