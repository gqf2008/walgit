import { useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { parsePatchFiles } from "@pierre/diffs";
import { FileDiff } from "@pierre/diffs/react";
import { api } from "../api";
import { useData } from "../data";
import { useRepo } from "./RepoLayout";
import { Box } from "../components/Layout";
import { relTime } from "../format";
import { Avatar } from "../components/CommitRow";
import { Linkified, Trailers } from "../components/CommitMessage";
import { useI18n } from "../i18n";

export function CommitPage() {
  const { t } = useI18n();
  const { full } = useRepo();
  const { sha = "" } = useParams();
  const data = useData(`commit:${full}:${sha}`, () => api.commit(full, sha), Infinity);
  const [split, setSplit] = useState(false);
  const files = useMemo(() => {
    if (!data.patch) return [];
    try {
      return parsePatchFiles(data.patch, sha).flatMap((p) => p.files);
    } catch (e) {
      console.error(e);
      return [];
    }
  }, [data, sha]);
  const { commit: c, stats } = data;
  const add = stats.reduce((n, s) => n + Math.max(0, s.additions), 0);
  const del = stats.reduce((n, s) => n + Math.max(0, s.deletions), 0);
  return (
    <>
      <Box className="commit-box">
        <div className="commit-title pad">
          <h2>{c.subject}</h2>
          {c.body && (
            <pre className="commit-body-full">
              <Linkified text={c.body} />
            </pre>
          )}
          <div className="row wrap gap">
            <Trailers repo={full} trailers={c.trailers ?? []} open />
            <span className="spacer" />
            <Link className="btn small" to={`/${full}/tree/${c.sha}`}>
              {t("commit.browseFiles")}
            </Link>
          </div>
        </div>
        <div className="commit-foot pad row wrap gap">
          <Avatar name={c.author} />
          <strong>{c.author}</strong>
          <span className="muted">
            {t("commit.authored", { time: relTime(c.author_date) })}
            {(c.committer !== c.author || c.commit_date !== c.author_date) && (
              <> · {t("commit.committedBy", { committer: c.committer, time: relTime(c.commit_date) })}</>
            )}
          </span>
          <span className="spacer" />
          <span className="muted small">
            {c.parents.length === 0 && t("commit.root")}
            {c.parents.length > 0 && (
              <>
                {c.parents.length === 1 ? t("commit.parent") : t("commit.parents")}{" "}
                {c.parents.map((p, i) => (
                  <span key={p}>
                    {i > 0 && " + "}
                    <Link to={`/${full}/commit/${p}`} className="sha">
                      {p.slice(0, 7)}
                    </Link>
                  </span>
                ))}
              </>
            )}
            {" · "}
            {t("commit.self")}{" "}
            <span className="sha">{c.sha.slice(0, 7)}</span>
          </span>
        </div>
      </Box>

      <div className="diffstat row wrap gap">
        <span>
          {t("commit.diffstat.showing")}
          <strong>{stats.length}</strong>
          {stats.length === 1 ? t("commit.diffstat.file") : t("commit.diffstat.files")}
          <strong className="add">{t("commit.diffstat.additions", { n: add })}</strong>
          <strong className="del">{t("commit.diffstat.deletions", { n: del })}</strong>
        </span>
        <span className="spacer" />
        <span className="seg">
          <button className={split ? "" : "active"} onClick={() => setSplit(false)}>
            {t("commit.unified")}
          </button>
          <button className={split ? "active" : ""} onClick={() => setSplit(true)}>
            {t("commit.split")}
          </button>
        </span>
      </div>
      <details className="box filelist">
        <summary className="box-header">{t("commit.filesChanged")}</summary>
        <ul className="list compact">
          {stats.map((s) => (
            <li key={s.path} className="row">
              <a href={`#d-${encodeURIComponent(s.path)}`}>{s.path}</a>
              <span className="spacer" />
              {s.additions < 0 ? (
                <span className="muted small">{t("commit.binary")}</span>
              ) : (
                <span className="small">
                  <span className="add">+{s.additions}</span> <span className="del">−{s.deletions}</span>
                </span>
              )}
            </li>
          ))}
        </ul>
      </details>

      {files.map((f, i) => (
        <div key={f.name + i} id={`d-${encodeURIComponent(f.name)}`} className="diff-file">
          <FileDiff fileDiff={f} options={{ diffStyle: split ? "split" : "unified", themeType: "light", overflow: "scroll" }} />
        </div>
      ))}
      {files.length === 0 && stats.length > 0 && (
        <Box>
          <pre className="pad code-block">{data.patch}</pre>
        </Box>
      )}
    </>
  );
}
