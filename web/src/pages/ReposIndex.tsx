import { Link } from "react-router-dom";
import { api } from "../api";
import { useData } from "../data";
import { Box } from "../components/Layout";
import { useI18n } from "../i18n";

/** Every repository on this host, grouped by owner (top-nav「仓库」lands here;
 * `/` is the landing page). Owners come from `/api/v1/owners`, each section
 * lists that owner's repos with the clone command, like the per-owner page. */
export function ReposIndex() {
  const { t } = useI18n();
  const owners = useData("owners", () => api.owners());
  return (
    <>
      <h1 className="page-title">{t("repos.title")}</h1>
      <p className="muted">{t("repos.all.subtitle")}</p>
      {owners.length === 0 && (
        <Box>
          <div className="muted pad">
            {t("repos.all.empty.pre")}
            <code>{location.origin}/owner/repository.git</code>
            {t("repos.all.empty.post")}
          </div>
        </Box>
      )}
      {owners.map((owner) => (
        <OwnerSection key={owner} owner={owner} />
      ))}
    </>
  );
}

function OwnerSection({ owner }: { owner: string }) {
  const repos = useData(`repos:${owner}`, () => api.repos(owner));
  if (repos.length === 0) return null;
  return (
    <Box
      title={
        <Link to={`/${owner}`} className="strong">
          {owner}
        </Link>
      }
    >
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
  );
}
