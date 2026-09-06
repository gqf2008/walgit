import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useI18n } from "../i18n";

/**
 * The first-run setup wizard (issue #70, D43). Served only while the instance
 * is in setup state (a `memory` store without the deliberate-use flag): every
 * other route answers 503, so this page IS the whole UI until the store and
 * the first admin are configured. Plain `fetch` to the open, data-free
 * `/api/v1/setup/*` surface — no SDK lane (no repo, no session exists yet).
 *
 * Flow: ① object storage (S3/R2 or GCS) → test connection → ② first admin
 * token → save → the server writes walgit.toml (comments preserved) and exits
 * 75 for the supervisor to restart; without a supervisor the page tells the
 * user to restart manually.
 */

interface SetupStatus {
  needs_setup: boolean;
  backend: string;
  auth_mode: string;
  can_save: boolean;
}

interface SetupPayload {
  backend: "s3" | "gcs";
  bucket: string;
  endpoint: string;
  region: string;
  access_key: string;
  secret_key: string;
  force_path_style: boolean;
}

type Phase = "form" | "testing" | "saving" | "saved";

export function SetupPage() {
  const { t } = useI18n();
  const [status, setStatus] = useState<SetupStatus | "404" | "loading">("loading");
  const [payload, setPayload] = useState<SetupPayload>({
    backend: "s3",
    bucket: "",
    endpoint: "",
    region: "auto",
    access_key: "",
    secret_key: "",
    force_path_style: true,
  });
  const [adminToken, setAdminToken] = useState("");
  const [adminPrincipal, setAdminPrincipal] = useState("");
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [restart, setRestart] = useState<"supervisor" | "manual" | null>(null);
  const [phase, setPhase] = useState<Phase>("form");

  // Load once: are we even in setup state?
  useEffect(() => {
    fetch("/api/v1/setup/status")
      .then(async (r) => {
        if (r.status === 404) setStatus("404");
        else setStatus((await r.json()) as SetupStatus);
      })
      .catch(() => setStatus("404"));
  }, []);

  const set = (k: keyof SetupPayload, v: string | boolean) =>
    setPayload((p) => ({ ...p, [k]: v }));

  const runTest = async () => {
    setPhase("testing");
    setTestResult(null);
    try {
      const r = await fetch("/api/v1/setup/test", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      if (r.ok) {
        const body = (await r.json()) as { ok: boolean; message: string };
        setTestResult({ ok: body.ok, message: body.message });
      } else if ((r.headers.get("content-type") ?? "").includes("application/json")) {
        // The most common failure (bad credentials / unreachable endpoint) is
        // a 400 + {ok:false, message} — show the message, not the JSON.
        const body = (await r.json()) as { ok?: boolean; message?: string };
        setTestResult({ ok: false, message: body.message ?? String(body) });
      } else {
        // Validation rejections answer text/plain with the reason.
        setTestResult({ ok: false, message: await r.text() });
      }
    } catch (e) {
      setTestResult({ ok: false, message: String(e) });
    } finally {
      setPhase("form");
    }
  };

  const runSave = async () => {
    setPhase("saving");
    setSaveError(null);
    try {
      const r = await fetch("/api/v1/setup/save", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          store: payload,
          admin_token: adminToken,
          admin_principal: adminPrincipal,
        }),
      });
      if (r.ok) {
        const body = (await r.json()) as {
          saved?: boolean;
          restart?: "supervisor" | "manual";
        };
        if (body.saved) {
          setRestart(body.restart ?? "manual");
          setPhase("saved");
          return;
        }
        setSaveError("server answered ok but did not save");
      } else {
        // The rejection paths answer text/plain with the reason.
        setSaveError(await r.text());
      }
      setPhase("form");
    } catch (e) {
      setSaveError(String(e));
      setPhase("form");
    }
  };

  if (status === "404") {
    return (
      <main className="setup">
        <div className="setup-card">
          <h1>{t("setup.title")}</h1>
          <p className="muted">{t("setup.already")}</p>
          <Link className="btn primary" to="/">
            {t("setup.open")}
          </Link>
        </div>
      </main>
    );
  }
  if (status === "loading") {
    return (
      <main className="setup">
        <div className="setup-card">
          <h1>{t("setup.title")}</h1>
          <p className="muted">…</p>
        </div>
      </main>
    );
  }

  const st = status;
  if (phase === "saved") {
    return (
      <main className="setup">
        <div className="setup-card">
          <h1>{t("setup.title")}</h1>
          <p className="setup-ok">
            {restart === "supervisor" ? t("setup.saved") : t("setup.saved.manual")}
          </p>
        </div>
      </main>
    );
  }

  return (
    <main className="setup">
      <div className="setup-card">
        <h1>{t("setup.title")}</h1>
        <p>{t("setup.lede")}</p>
        <div className="setup-warn">{t("setup.memory.warn")}</div>

        <h2>{t("setup.step.store")}</h2>
        <div className="setup-grid">
          <label className="setup-field">
            <span>{t("setup.backend")}</span>
            <select
              value={payload.backend}
              onChange={(e) => set("backend", e.target.value)}
            >
              <option value="s3">{t("setup.backend.s3")}</option>
              <option value="gcs">{t("setup.backend.gcs")}</option>
            </select>
          </label>
          <label className="setup-field">
            <span>{t("setup.bucket")}</span>
            <input
              value={payload.bucket}
              onChange={(e) => set("bucket", e.target.value)}
              placeholder="walgit"
            />
          </label>
          <label className="setup-field">
            <span>{t("setup.endpoint")}</span>
            <input
              value={payload.endpoint}
              onChange={(e) => set("endpoint", e.target.value)}
            />
            <small className="muted">
              {payload.backend === "s3" ? t("setup.endpoint.s3.hint") : t("setup.endpoint.gcs.hint")}
            </small>
          </label>
          {payload.backend === "s3" ? (
            <>
              <label className="setup-field">
                <span>{t("setup.region")}</span>
                <input value={payload.region} onChange={(e) => set("region", e.target.value)} />
                <small className="muted">{t("setup.region.hint")}</small>
              </label>
              <label className="setup-field">
                <span>{t("setup.access_key")}</span>
                <input
                  value={payload.access_key}
                  onChange={(e) => set("access_key", e.target.value)}
                  autoComplete="off"
                />
              </label>
              <label className="setup-field">
                <span>{t("setup.secret_key")}</span>
                <input
                  type="password"
                  value={payload.secret_key}
                  onChange={(e) => set("secret_key", e.target.value)}
                  autoComplete="new-password"
                />
                <small className="muted">{t("setup.creds.hint")}</small>
              </label>
              <label className="setup-check">
                <input
                  type="checkbox"
                  checked={payload.force_path_style}
                  onChange={(e) => set("force_path_style", e.target.checked)}
                />
                {t("setup.force_path_style")}
              </label>
            </>
          ) : (
            <p className="muted">{t("setup.gcs.adc")}</p>
          )}
        </div>

        <p>
          <button className="btn primary" type="button" disabled={phase !== "form"} onClick={runTest}>
            {phase === "testing" ? t("setup.testing") : t("setup.test")}
          </button>
        </p>
        {testResult && (
          <div className={testResult.ok ? "setup-ok" : "setup-warn"}>
            {testResult.ok ? t("setup.test.ok") : `${t("setup.test.fail")} ${testResult.message}`}
          </div>
        )}

        <h2>{t("setup.step.admin")}</h2>
        <div className="setup-grid">
          <label className="setup-field">
            <span>{t("setup.admin.token")}</span>
            <input
              type="password"
              value={adminToken}
              onChange={(e) => setAdminToken(e.target.value)}
              autoComplete="new-password"
            />
            <small className="muted">{t("setup.admin.token.hint")}</small>
          </label>
          <label className="setup-field">
            <span>{t("setup.admin.principal")}</span>
            <input
              value={adminPrincipal}
              onChange={(e) => setAdminPrincipal(e.target.value)}
              placeholder="admin"
            />
          </label>
        </div>

        <p>
          <button
            className="btn primary"
            type="button"
            disabled={phase !== "form" || !payload.bucket.trim() || !st.can_save}
            onClick={runSave}
          >
            {phase === "saving" ? t("setup.saving") : t("setup.save")}
          </button>
        </p>
        {!st.can_save && (
          <p className="muted">{t("setup.nosave")}</p>
        )}
        {saveError && <div className="setup-warn">{`${t("setup.save.fail")} ${saveError}`}</div>}
      </div>
    </main>
  );
}
