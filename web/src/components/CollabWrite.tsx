import { useCallback, useState } from "react";
import { api } from "../api";
import { invalidate } from "../data";
import { ed25519Supported, publicKeyB64, signCanonical } from "../collab";

/**
 * D1 browser write box: sign in as the session principal, self-register the
 * browser Ed25519 public key through the thin API, and post signed entries
 * (issue / comment / review / status / patch). Entries are verified by the
 * aggregation exactly like CLI/agent writes.
 */

export interface CollabWriteProps {
  full: string;
  /** Thread id this entry joins (new threads use a fresh uuid). */
  id: string;
  /** Previous entry oid in the thread ("" for a root entry). */
  parent: string;
  onPosted?: () => void;
}

type Kind = "issue" | "comment" | "review" | "status" | "patch";

export function CollabWriteBox({ full, id, parent, onPosted }: CollabWriteProps) {
  const [ready, setReady] = useState<string | null>(null); // principal when the browser key is registered
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [kind, setKind] = useState<Kind>(parent === "" ? "issue" : "comment");
  const [text, setText] = useState("");
  const [decision, setDecision] = useState<"approve" | "request_changes" | "comment">("approve");
  const [status, setStatus] = useState<"open" | "closed" | "merged">("closed");
  const [base, setBase] = useState("refs/heads/main");
  const [head, setHead] = useState("");

  const enable = useCallback(async (): Promise<string | null> => {
    setBusy(true);
    setError(null);
    try {
      if (!ed25519Supported()) {
        setError("This browser has no WebCrypto Ed25519 support — use the walgit collab CLI to sign entries.");
        return null;
      }
      const me = await api.me();
      if (me.anonymous) {
        setError("Signed out — sign in to participate in the collaboration layer.");
        return null;
      }
      const publicKey = await publicKeyB64();
      // Self-registration is idempotent in effect (re-registering the same key
      // just appends a log entry; the aggregation reads the latest key).
      await api.collab(full).registerPrincipal(me.principal, publicKey);
      invalidate(`collab:${full}`);
      setReady(me.principal);
      return me.principal;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return null;
    } finally {
      setBusy(false);
    }
  }, [full]);

  const post = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const principal = ready ?? (await enable());
      if (!principal) return; // enable() set the error
      const body: Record<string, unknown> =
        kind === "issue"
          ? { title: text.split("\n")[0] || "untitled", text }
          : kind === "comment"
            ? { text }
            : kind === "review"
              ? { decision }
              : kind === "status"
                ? { status }
                : { message: text };
      const refs = kind === "patch" ? { base, head } : undefined;
      const entry = await api.collabBuildEntry(full, {
        principal,
        kind,
        id,
        actor: principal,
        parent,
        refs,
        body,
        sign: signCanonical,
      });
      await api.collab(full).post(entry);
      invalidate(`collab:${full}`);
      setText("");
      onPosted?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [ready, enable, kind, text, decision, status, base, head, full, id, parent, onPosted]);

  if (!ready) {
    return (
      <div className="pad">
        {error && <div className="muted" style={{ color: "var(--danger, #f85149)" }}>{error}</div>}
        <button className="btn" disabled={busy} onClick={enable}>
          {busy ? "Setting up…" : "Enable my key & register"}
        </button>
        <span className="muted"> — generate an Ed25519 keypair in this browser, self-register the public key, and post signed entries.</span>
      </div>
    );
  }
  return (
    <div className="pad">
      <div className="row gap" style={{ alignItems: "center" }}>
        <strong>{ready}</strong>
        <select value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
          <option value="issue">issue</option>
          <option value="comment">comment</option>
          <option value="review">review</option>
          <option value="status">status</option>
          <option value="patch">patch</option>
        </select>
        {kind === "review" && (
          <select value={decision} onChange={(e) => setDecision(e.target.value as typeof decision)}>
            <option value="approve">approve</option>
            <option value="request_changes">request_changes</option>
            <option value="comment">comment</option>
          </select>
        )}
        {kind === "status" && (
          <select value={status} onChange={(e) => setStatus(e.target.value as typeof status)}>
            <option value="closed">closed</option>
            <option value="merged">merged</option>
            <option value="open">open</option>
          </select>
        )}
      </div>
      {kind === "patch" && (
        <div className="row gap">
          <input value={base} onChange={(e) => setBase(e.target.value)} placeholder="base ref" />
          <input value={head} onChange={(e) => setHead(e.target.value)} placeholder="head ref" />
        </div>
      )}
      <textarea
        className="collab-body"
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder={kind === "issue" ? "First line = title, rest = body" : kind === "review" || kind === "status" ? "optional note" : "Write…"}
        rows={4}
      />
      <div className="row gap">
        <button className="btn primary" disabled={busy || (kind === "patch" && !head)} onClick={post}>
          {busy ? "Posting…" : `Post ${kind}`}
        </button>
        {error && <span className="muted" style={{ color: "var(--danger, #f85149)" }}>{error}</span>}
      </div>
    </div>
  );
}
