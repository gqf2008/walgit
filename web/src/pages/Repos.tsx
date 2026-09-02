import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useData } from "../data";
import { Box } from "../components/Layout";
import { useI18n } from "../i18n";

export function Repos() {
  const { t } = useI18n();
  const { owner = "" } = useParams();
  const repos = useData(`repos:${owner}`, () => api.repos(owner));
  return (
    <>
      <h1 className="page-title">
        <Link to="/">{t("repos.title")}</Link> <span className="muted">/</span> {owner}
      </h1>
      <Box>
        {repos.length === 0 && (
          <div className="muted pad">
            {t("repos.empty.pre")}
            <code>{owner}</code>
            {t("repos.empty.mid")}
            <code>{location.origin}/{owner}/repository.git</code>
            {t("repos.empty.post")}
          </div>
        )}
        <ul className="list">
          {repos.map((r) => (
            <li key={r}>
              <Link to={`/${owner}/${r}`} className="strong">
                {owner}/{r}
              </Link>
              <div className="muted small">
                <code>
                  git -c transfer.bundleURI=true clone {location.origin}/{owner}/{r}.git
                </code>
              </div>
            </li>
          ))}
        </ul>
      </Box>
    </>
  );
}
