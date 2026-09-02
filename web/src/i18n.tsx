import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

/**
 * UI internationalization (issue #39): a zero-dependency dictionary lookup.
 * `en` is the source language and the fallback; the other dictionaries must
 * cover every key (`Record<I18nKey, string>` — a missing key is a compile
 * error, so new UI text cannot silently ship untranslated).
 *
 * Values stored in git (status slugs, entry kinds) are DATA, not UI text:
 * they are translated at render time through the label helpers at the bottom
 * and never rewritten in storage.
 */

export type Lang = "en" | "zh-CN" | "zh-TW";

export const LANGS: { value: Lang; label: string }[] = [
  { value: "en", label: "English" },
  { value: "zh-CN", label: "简体中文" },
  { value: "zh-TW", label: "繁體中文" },
];

const en = {
  "lang.switch": "Language",

  // Repo chrome
  "tab.code": "Code",
  "tab.commits": "Commits",
  "tab.wal": "WAL",
  "tab.collab": "Collab",
  "tab.settings": "Settings",
  "clone.button": "Clone",
  "back.collab": "← collab",

  // Collab index
  "collab.title": "D1 collaboration",
  "collab.threads": "Threads",
  "collab.prs": "Pull requests",
  "collab.entries": "Entries",
  "collab.entries.summary": "{total} · ✓ {verified} verified · {unverified} unverified · {missing} missing keys",
  "collab.board.link": "work-unit board →",
  "collab.newThread": "New thread",
  "collab.noThreads": "No threads yet — post the first entry above.",
  "collab.noPrs": "No patches.",
  "collab.byActor": "By actor",
  "collab.byKind": "By kind",
  "collab.th.id": "id",
  "collab.th.kinds": "kinds",
  "collab.th.entries": "entries",
  "collab.th.verified": "verified",
  "collab.th.lastActivity": "last activity",
  "collab.th.baseHead": "base → head",
  "collab.th.status": "status",
  "collab.th.approvals": "approvals",
  "collab.th.merge": "merge",
  "collab.merge.allowed": "allowed",

  // Board
  "board.title": "Work-unit board",
  "board.explainer":
    "The threads of refs/collab/* projected under the board definition at .walgit/board.toml (the same build_board the CLI renders offline — no board state exists anywhere). Moving a card posts a signed status entry.",
  "board.moveTo": "move to…",
  "board.moving": "moving…",
  "board.card.meta": "{actor} · {entries} entries · {verified} verified",
  "board.card.unverified": " · {n} unverified",
  "board.card.merge.allowed": " · merge allowed",
  "board.card.merge.blocked": " · merge blocked",

  // Thread
  "thread.title": "Thread {id}",
  "thread.write": "Write",
  "pr.baseHead": "base → head",
  "pr.status": "status",
  "pr.reviews": "reviews",
  "pr.approvals": "approvals",
  "pr.approvals.value": "{human} human · {unverified} unverified",
  "pr.mergeRule": "merge rule",
  "pr.showDiff": "Show diff {base} → {head}",
  "pr.diff": "Diff {base} → {head}",
  "pr.noChanges": "No file changes.",
  "entry.verified": "✓ verified",
  "entry.unverified": "✗ unverified",
  "entry.claim.title": "claim {task} · attempt {attempt}",
  "entry.result.title": "result {task} · attempt {attempt}",
  "entry.result.meta": "{ref} @ {commit} · exit {code} · {ms} ms · log sha {sha}",
  "entry.issue.untitled": "(issue)",

  // CollabWrite
  "write.enable": "Enable my key & register",
  "write.enabling": "Setting up…",
  "write.enable.hint":
    " — generate an Ed25519 keypair in this browser, self-register the public key, and post signed entries.",
  "write.post": "Post {kind}",
  "write.posting": "Posting…",
  "write.ph.issue": "First line = title, rest = body",
  "write.ph.note": "optional note",
  "write.ph.write": "Write…",
  "write.baseRef": "base ref",
  "write.headRef": "head ref",
  "write.err.noWebCrypto": "This browser has no WebCrypto Ed25519 support — use the walgit collab CLI to sign entries.",
  "write.err.signedOut": "Signed out — sign in to participate in the collaboration layer.",
  "write.err.untitled": "untitled",

  // Entry kinds (data values, translated at render time)
  "kind.issue": "issue",
  "kind.comment": "comment",
  "kind.review": "review",
  "kind.status": "status",
  "kind.patch": "patch",
  "kind.ci_claim": "ci_claim",
  "kind.ci_result": "ci_result",
  "kind.merge_result": "merge_result",

  // Status slugs (data values)
  "status.open": "open",
  "status.in-progress": "in-progress",
  "status.needs-review": "needs-review",
  "status.blocked": "blocked",
  "status.needs-human": "needs-human",
  "status.merged": "merged",
  "status.closed": "closed",

  // Review decisions (data values)
  "review.approve": "approve",
  "review.request_changes": "request_changes",
  "review.comment": "comment",
} as const;

export type I18nKey = keyof typeof en;

const zhCN: Record<I18nKey, string> = {
  "lang.switch": "语言",

  "tab.code": "代码",
  "tab.commits": "提交",
  "tab.wal": "WAL",
  "tab.collab": "协作",
  "tab.settings": "设置",
  "clone.button": "克隆",
  "back.collab": "← 协作",

  "collab.title": "D1 协作",
  "collab.threads": "讨论串",
  "collab.prs": "拉取请求",
  "collab.entries": "条目",
  "collab.entries.summary": "{total} 条 · ✓ {verified} 已验证 · {unverified} 未验证 · {missing} 个缺失密钥",
  "collab.board.link": "工作单元看板 →",
  "collab.newThread": "新讨论串",
  "collab.noThreads": "还没有讨论串——在上方发布第一条条目。",
  "collab.noPrs": "还没有补丁。",
  "collab.byActor": "按签名者",
  "collab.byKind": "按类型",
  "collab.th.id": "id",
  "collab.th.kinds": "类型",
  "collab.th.entries": "条目数",
  "collab.th.verified": "已验证",
  "collab.th.lastActivity": "最后活动",
  "collab.th.baseHead": "base → head",
  "collab.th.status": "状态",
  "collab.th.approvals": "通过数",
  "collab.th.merge": "合并",
  "collab.merge.allowed": "允许",

  "board.title": "工作单元看板",
  "board.explainer":
    "refs/collab/* 上的讨论串，按 .walgit/board.toml 的列定义投影而成（与 CLI 离线运行的是同一个 build_board——看板状态不存在任何地方）。移动卡片即追加一条签名 status 条目。",
  "board.moveTo": "移动到…",
  "board.moving": "移动中…",
  "board.card.meta": "{actor} · {entries} 条条目 · {verified} 已验证",
  "board.card.unverified": " · {n} 条未验证",
  "board.card.merge.allowed": " · 允许合并",
  "board.card.merge.blocked": " · 禁止合并",

  "thread.title": "讨论串 {id}",
  "thread.write": "撰写",
  "pr.baseHead": "base → head",
  "pr.status": "状态",
  "pr.reviews": "评审",
  "pr.approvals": "通过情况",
  "pr.approvals.value": "{human} 人通过 · {unverified} 条未验证",
  "pr.mergeRule": "合并规则",
  "pr.showDiff": "显示差异 {base} → {head}",
  "pr.diff": "差异 {base} → {head}",
  "pr.noChanges": "没有文件变更。",
  "entry.verified": "✓ 已验证",
  "entry.unverified": "✗ 未验证",
  "entry.claim.title": "认领 {task} · 第 {attempt} 次尝试",
  "entry.result.title": "结果 {task} · 第 {attempt} 次尝试",
  "entry.result.meta": "{ref} @ {commit} · 退出码 {code} · {ms} ms · 日志 sha {sha}",
  "entry.issue.untitled": "（议题）",

  "write.enable": "启用我的密钥并注册",
  "write.enabling": "设置中…",
  "write.enable.hint": "——在此浏览器中生成 Ed25519 密钥对，自助注册公钥，即可发布签名条目。",
  "write.post": "发布{kind}",
  "write.posting": "发布中…",
  "write.ph.issue": "第一行 = 标题，其余 = 正文",
  "write.ph.note": "可选备注",
  "write.ph.write": "写点什么…",
  "write.baseRef": "base ref",
  "write.headRef": "head ref",
  "write.err.noWebCrypto": "此浏览器不支持 WebCrypto Ed25519——请改用 walgit collab CLI 签名条目。",
  "write.err.signedOut": "已登出——登录后才能参与协作层。",
  "write.err.untitled": "无标题",

  "kind.issue": "议题",
  "kind.comment": "评论",
  "kind.review": "评审",
  "kind.status": "状态",
  "kind.patch": "补丁",
  "kind.ci_claim": "CI 认领",
  "kind.ci_result": "CI 结果",
  "kind.merge_result": "合并结果",

  "status.open": "开放",
  "status.in-progress": "进行中",
  "status.needs-review": "待评审",
  "status.blocked": "受阻",
  "status.needs-human": "需要人工",
  "status.merged": "已合并",
  "status.closed": "已关闭",

  "review.approve": "通过",
  "review.request_changes": "要求修改",
  "review.comment": "评论",
};

const zhTW: Record<I18nKey, string> = {
  "lang.switch": "語言",

  "tab.code": "程式碼",
  "tab.commits": "提交",
  "tab.wal": "WAL",
  "tab.collab": "協作",
  "tab.settings": "設定",
  "clone.button": "複製",
  "back.collab": "← 協作",

  "collab.title": "D1 協作",
  "collab.threads": "討論串",
  "collab.prs": "提取請求",
  "collab.entries": "條目",
  "collab.entries.summary": "{total} 條 · ✓ {verified} 已驗證 · {unverified} 未驗證 · {missing} 個遺失金鑰",
  "collab.board.link": "工作單元看板 →",
  "collab.newThread": "新討論串",
  "collab.noThreads": "還沒有討論串——在上方發佈第一條條目。",
  "collab.noPrs": "還沒有補丁。",
  "collab.byActor": "按簽署者",
  "collab.byKind": "按類型",
  "collab.th.id": "id",
  "collab.th.kinds": "類型",
  "collab.th.entries": "條目數",
  "collab.th.verified": "已驗證",
  "collab.th.lastActivity": "最後活動",
  "collab.th.baseHead": "base → head",
  "collab.th.status": "狀態",
  "collab.th.approvals": "通過數",
  "collab.th.merge": "合併",
  "collab.merge.allowed": "允許",

  "board.title": "工作單元看板",
  "board.explainer":
    "refs/collab/* 上的討論串，按 .walgit/board.toml 的欄定義投影而成（與 CLI 離線執行的是同一個 build_board——看板狀態不存在任何地方）。移動卡片即追加一條簽名 status 條目。",
  "board.moveTo": "移動到…",
  "board.moving": "移動中…",
  "board.card.meta": "{actor} · {entries} 條條目 · {verified} 已驗證",
  "board.card.unverified": " · {n} 條未驗證",
  "board.card.merge.allowed": " · 允許合併",
  "board.card.merge.blocked": " · 禁止合併",

  "thread.title": "討論串 {id}",
  "thread.write": "撰寫",
  "pr.baseHead": "base → head",
  "pr.status": "狀態",
  "pr.reviews": "審查",
  "pr.approvals": "通過情況",
  "pr.approvals.value": "{human} 人通過 · {unverified} 條未驗證",
  "pr.mergeRule": "合併規則",
  "pr.showDiff": "顯示差異 {base} → {head}",
  "pr.diff": "差異 {base} → {head}",
  "pr.noChanges": "沒有檔案變更。",
  "entry.verified": "✓ 已驗證",
  "entry.unverified": "✗ 未驗證",
  "entry.claim.title": "認領 {task} · 第 {attempt} 次嘗試",
  "entry.result.title": "結果 {task} · 第 {attempt} 次嘗試",
  "entry.result.meta": "{ref} @ {commit} · 結束碼 {code} · {ms} ms · 日誌 sha {sha}",
  "entry.issue.untitled": "（議題）",

  "write.enable": "啟用我的金鑰並註冊",
  "write.enabling": "設定中…",
  "write.enable.hint": "——在此瀏覽器產生 Ed25519 金鑰對，自助註冊公鑰，即可發佈簽名條目。",
  "write.post": "發佈{kind}",
  "write.posting": "發佈中…",
  "write.ph.issue": "第一行 = 標題，其餘 = 內文",
  "write.ph.note": "可選備註",
  "write.ph.write": "寫點什麼…",
  "write.baseRef": "base ref",
  "write.headRef": "head ref",
  "write.err.noWebCrypto": "此瀏覽器不支援 WebCrypto Ed25519——請改用 walgit collab CLI 簽名條目。",
  "write.err.signedOut": "已登出——登入後才能參與協作層。",
  "write.err.untitled": "無標題",

  "kind.issue": "議題",
  "kind.comment": "評論",
  "kind.review": "審查",
  "kind.status": "狀態",
  "kind.patch": "補丁",
  "kind.ci_claim": "CI 認領",
  "kind.ci_result": "CI 結果",
  "kind.merge_result": "合併結果",

  "status.open": "開放",
  "status.in-progress": "進行中",
  "status.needs-review": "待審查",
  "status.blocked": "受阻",
  "status.needs-human": "需要人工",
  "status.merged": "已合併",
  "status.closed": "已關閉",

  "review.approve": "通過",
  "review.request_changes": "要求修改",
  "review.comment": "評論",
};

const DICTS: Record<Lang, Record<I18nKey, string>> = { en, "zh-CN": zhCN, "zh-TW": zhTW };

export type TFunc = (key: I18nKey, params?: Record<string, string | number>) => string;

interface I18nCtx {
  lang: Lang;
  setLang: (l: Lang) => void;
  t: TFunc;
}

const Ctx = createContext<I18nCtx | null>(null);

const STORE_KEY = "walgit.lang";

function detect(): Lang {
  try {
    const stored = localStorage.getItem(STORE_KEY);
    if (stored === "en" || stored === "zh-CN" || stored === "zh-TW") return stored;
  } catch {
    // private mode / blocked storage: fall through to navigator
  }
  const navs: string[] = typeof navigator !== "undefined" ? [navigator.language, ...navigator.languages] : [];
  for (const n of navs) {
    if (/zh[-_](TW|HK|MO)\b/i.test(n) || /zh[-_]Hant/i.test(n)) return "zh-TW";
    if (/^zh\b/i.test(n)) return "zh-CN";
  }
  return "en";
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(detect);
  useEffect(() => {
    document.documentElement.lang = lang;
    try {
      localStorage.setItem(STORE_KEY, lang);
    } catch {
      // storage may be unavailable; the choice simply lasts for this tab
    }
  }, [lang]);
  const value = useMemo<I18nCtx>(() => {
    const dict = DICTS[lang];
    const t: TFunc = (key, params) => {
      let s: string = dict[key] ?? en[key] ?? key;
      if (params) for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v));
      return s;
    };
    return { lang, setLang: setLangState, t };
  }, [lang]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useI18n(): I18nCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error("useI18n outside I18nProvider");
  return c;
}

/** The language switcher in the global top bar. */
export function LangSwitch() {
  const { lang, setLang, t } = useI18n();
  return (
    <select
      className="lang-switch"
      aria-label={t("lang.switch")}
      title={t("lang.switch")}
      value={lang}
      onChange={(e) => setLang(e.target.value as Lang)}
    >
      {LANGS.map((l) => (
        <option key={l.value} value={l.value}>
          {l.label}
        </option>
      ))}
    </select>
  );
}

/** Data values (kinds/statuses/decisions) are stored in English; translate at
    render time and pass unknown future values through unchanged. */
const KIND_KEYS: Record<string, I18nKey> = {
  issue: "kind.issue",
  comment: "kind.comment",
  review: "kind.review",
  status: "kind.status",
  patch: "kind.patch",
  ci_claim: "kind.ci_claim",
  ci_result: "kind.ci_result",
  merge_result: "kind.merge_result",
};
const STATUS_KEYS: Record<string, I18nKey> = {
  open: "status.open",
  "in-progress": "status.in-progress",
  "needs-review": "status.needs-review",
  blocked: "status.blocked",
  "needs-human": "status.needs-human",
  merged: "status.merged",
  closed: "status.closed",
};
const DECISION_KEYS: Record<string, I18nKey> = {
  approve: "review.approve",
  request_changes: "review.request_changes",
  comment: "review.comment",
};

export function kindLabel(t: TFunc, kind: string): string {
  const k = KIND_KEYS[kind];
  return k ? t(k) : kind;
}
export function statusLabel(t: TFunc, status: string): string {
  const k = STATUS_KEYS[status];
  return k ? t(k) : status;
}
export function decisionLabel(t: TFunc, decision: string): string {
  const k = DECISION_KEYS[decision];
  return k ? t(k) : decision;
}
