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

  // Guide page ("了解 D1 协作", issue #41)
  "guide.link": "What is this?",
  "guide.title": "Collaboration, without a database",
  "guide.lede":
    "Everything on the collab pages — issues, the board, CI results — is derived live from signed entries stored as git refs in this repository. No database sits behind them, only public rules every client applies identically. Three mental models cover all of it.",
  "guide.s1.title": "1 · An issue is an append-only chain of signed entries",
  "guide.s1.body":
    "Nobody ever edits an issue. Each participant appends one signed entry, chained to the previous by parent. The current state is a replay of the chain — the last matching entry wins.",
  "guide.s1.live": "This is a real thread from this repository:",
  "guide.s1.sample": "(illustrative — this repository has no multi-entry thread yet)",
  "guide.s1.note":
    "Why append-only? The only durable state is an append-only log in the object store, and instances are disposable. Collaboration survives only if anyone can replay and verify it: every entry verifies on its own, the whole chain recomputes anywhere.",
  "guide.openThread": "open this thread →",
  "guide.s2.title": "2 · The board exists nowhere — it is computed",
  "guide.s2.body":
    "Same refs, same .walgit/board.toml, same ordering rule (last activity, then id): your browser and any other client each compute the board independently and arrive at byte-identical results. Not sync — a deterministic function.",
  "guide.s2.here": "This page",
  "guide.s2.here.sub": "rendered by this SPA",
  "guide.s2.any": "Any client",
  "guide.s2.any.sub": "walgit CLI · another browser · the API",
  "guide.s2.equiv": "byte-identical",
  "guide.s2.note":
    "The shift: in centralized systems you trust the platform; here you trust the rule — anyone starting from the same refs must compute the same board.",
  "guide.s3.title": "3 · Two runners may both run — only one counts",
  "guide.s3.body":
    "CI has no scheduler. Any runner that sees an untested commit claims it with a ci_claim entry. Simultaneous claims are a legal race: both may execute (at-least-once), but one deterministic winner rule yields exactly one effective result.",
  "guide.s3.step1": "Both claim",
  "guide.s3.step1.sub": "two runners post ci_claim for the same commit",
  "guide.s3.step2": "Both execute",
  "guide.s3.step2.sub": "at-least-once — the price of having no scheduler",
  "guide.s3.step3": "One rule picks the winner",
  "guide.s3.step3.sub": "winner = min(ts, actor, oid) — exactly one effective result",
  "guide.s3.note":
    "Why allow duplicate runs? Deduplication needs a central scheduler — exactly what this design removes. The cost is an occasional extra (idempotent) build; the gain: any machine can donate CI capacity, and everything stays auditable.",
  "guide.bugs.title": "Looks like a bug — isn't",
  "guide.bugs.lede": "Decentralization makes consistency visible instead of hiding it. Four sights you will meet:",
  "guide.bug1.sight": "The same task ran twice",
  "guide.bug1.truth":
    "A legal race. Two runners claimed in the same window; the winner rule picked the single effective result, the other is kept for audit.",
  "guide.bug2.sight": "A task is claimed but no result appears",
  "guide.bug2.truth":
    "The runner may have died. Claims expire (TTL); another runner re-claims the same attempt — nothing is lost.",
  "guide.bug3.sight": "A card's position on the board jumped",
  "guide.bug3.truth":
    "Ordering is (last activity, id). Two activities in the same second are ordered by id — a deterministic rule, not a rendering glitch.",
  "guide.bug4.sight": "A result “disappeared”",
  "guide.bug4.truth":
    "It merely isn't effective. Every entry still exists on the refs; existence is irrevocable, effectiveness is the rule's choice.",
  "guide.cmds.title": "When in doubt — three commands",
  "guide.cmd1.what": "replay any thread: every entry with its signature and parent link",
  "guide.cmd2.what": "the board, computed offline — byte-identical to this page",
  "guide.cmd3.what": "the global summary: threads, verification, activity, CI section",
  "guide.protocol": "The normative rules live in docs/D1_CI_PROTOCOL.md and AGENTS.md §2.",
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

  "guide.link": "这是什么？",
  "guide.title": "协作，但没有数据库",
  "guide.lede":
    "协作页面上的一切——议题、看板、CI 结果——都是从本仓库 git refs 上存储的签名条目实时推导出来的。它们背后没有数据库，只有一套每个客户端都一致执行的公开规则。三个心智模型即可覆盖全部。",
  "guide.s1.title": "1 · 议题 = 一条只能追加的签名条目链",
  "guide.s1.body":
    "没有人“编辑”议题。每个参与者追加一条签名条目，用 parent 指向上一条。当前状态是对这条链的重放——最后一条匹配的条目生效。",
  "guide.s1.live": "这是本仓库的一条真实讨论串：",
  "guide.s1.sample": "（示意——本仓库还没有多条目的讨论串）",
  "guide.s1.note":
    "为什么只能追加？唯一持久状态是对象存储里的 append-only 日志，实例是一次性的。协作要活下来，只能让任何人都能重放并验证：每条条目独立可验证，整条链在任意机器上重算。",
  "guide.openThread": "打开此讨论串 →",
  "guide.s2.title": "2 · 看板不存在于任何地方——它是算出来的",
  "guide.s2.body":
    "同样的 refs、同样的 .walgit/board.toml、同样的排序规则（最后活动，再按 id）：你的浏览器和任意其他客户端各自独立计算，得到字节一致的结果。不是同步——是确定性函数。",
  "guide.s2.here": "本页面",
  "guide.s2.here.sub": "由本 SPA 渲染",
  "guide.s2.any": "任意客户端",
  "guide.s2.any.sub": "walgit CLI · 另一个浏览器 · API",
  "guide.s2.equiv": "字节一致",
  "guide.s2.note":
    "转变在于：中心化系统里你信任平台；这里你信任规则——任何人从同样的 refs 出发，必须算出同样的看板。",
  "guide.s3.title": "3 · 两个 runner 可能都跑了——但只算一个",
  "guide.s3.body":
    "CI 没有调度器。任何看到未测试提交的 runner 用 ci_claim 条目认领它。同时认领是合法竞态：两边都可能执行（至少一次），但一条确定性的 winner 规则让恰好一个结果生效。",
  "guide.s3.step1": "都认领",
  "guide.s3.step1.sub": "两个 runner 对同一提交发布 ci_claim",
  "guide.s3.step2": "都执行",
  "guide.s3.step2.sub": "至少一次——没有调度器的代价",
  "guide.s3.step3": "一条规则判定 winner",
  "guide.s3.step3.sub": "winner = min(ts, actor, oid)——恰好一个结果生效",
  "guide.s3.note":
    "为什么允许重复执行？去重需要中心调度器——这正是本设计去掉的东西。代价是偶尔多跑一次（幂等的）构建；收益是任何机器都能贡献 CI 算力，且一切可审计。",
  "guide.bugs.title": "看起来像 bug——其实不是",
  "guide.bugs.lede": "去中心化把一致性摆到明面上，而不是藏起来。你会遇到的四种景象：",
  "guide.bug1.sight": "同一个任务跑了两次",
  "guide.bug1.truth": "合法竞态。两个 runner 在同一窗口认领；winner 规则选出唯一生效结果，另一条保留供审计。",
  "guide.bug2.sight": "任务被认领了却迟迟没结果",
  "guide.bug2.truth": "runner 可能挂了。认领会过期（TTL）；另一个 runner 会重新认领同一次尝试——不会丢。",
  "guide.bug3.sight": "卡片在看板上的位置跳了",
  "guide.bug3.truth": "排序是（最后活动，id）。同一秒内的两次活动按 id 排序——确定性规则，不是渲染故障。",
  "guide.bug4.sight": "结果“消失”了",
  "guide.bug4.truth": "它只是不生效。每条条目都还在 refs 上；存在不可撤销，生效是规则的选择。",
  "guide.cmds.title": "不确定时——三条命令",
  "guide.cmd1.what": "重放任意讨论串：每条条目都带着签名和父链",
  "guide.cmd2.what": "离线算出的看板——与本页字节一致",
  "guide.cmd3.what": "全局概览：讨论串、验证情况、活跃度、CI 小节",
  "guide.protocol": "规范性规则见 docs/D1_CI_PROTOCOL.md 与 AGENTS.md §2。",
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

  "guide.link": "這是什麼？",
  "guide.title": "協作，但沒有資料庫",
  "guide.lede":
    "協作頁面上的一切——議題、看板、CI 結果——都是從本倉庫 git refs 上儲存的簽名條目即時推導出來的。背後沒有資料庫，只有一套每個用戶端都一致執行的公開規則。三個心智模型即可涵蓋全部。",
  "guide.s1.title": "1 · 議題 = 一條只能追加的簽名條目鏈",
  "guide.s1.body":
    "沒有人「編輯」議題。每個參與者追加一條簽名條目，用 parent 指向上一條。目前狀態是對這條鏈的重放——最後一條匹配的條目生效。",
  "guide.s1.live": "這是本倉庫的一條真實討論串：",
  "guide.s1.sample": "（示意——本倉庫還沒有多條目的討論串）",
  "guide.s1.note":
    "為什麼只能追加？唯一持久狀態是物件儲存裡的 append-only 日誌，實例是一次性的。協作要活下來，只能讓任何人都能重放並驗證：每條條目獨立可驗證，整條鏈在任意機器上重算。",
  "guide.openThread": "開啟此討論串 →",
  "guide.s2.title": "2 · 看板不存在於任何地方——它是算出來的",
  "guide.s2.body":
    "同樣的 refs、同樣的 .walgit/board.toml、同樣的排序規則（最後活動，再按 id）：你的瀏覽器和任意其他用戶端各自獨立計算，得到位元組一致的結果。不是同步——是確定性函式。",
  "guide.s2.here": "本頁面",
  "guide.s2.here.sub": "由本 SPA 算繪",
  "guide.s2.any": "任意用戶端",
  "guide.s2.any.sub": "walgit CLI · 另一個瀏覽器 · API",
  "guide.s2.equiv": "位元組一致",
  "guide.s2.note":
    "轉變在於：中心化系統裡你信任平台；這裡你信任規則——任何人從同樣的 refs 出發，必須算出同樣的看板。",
  "guide.s3.title": "3 · 兩個 runner 可能都跑了——但只算一個",
  "guide.s3.body":
    "CI 沒有排程器。任何看到未測試提交的 runner 用 ci_claim 條目認領它。同時認領是合法競態：兩邊都可能執行（至少一次），但一條確定性的 winner 規則讓恰好一個結果生效。",
  "guide.s3.step1": "都認領",
  "guide.s3.step1.sub": "兩個 runner 對同一提交發佈 ci_claim",
  "guide.s3.step2": "都執行",
  "guide.s3.step2.sub": "至少一次——沒有排程器的代價",
  "guide.s3.step3": "一條規則判定 winner",
  "guide.s3.step3.sub": "winner = min(ts, actor, oid)——恰好一個結果生效",
  "guide.s3.note":
    "為什麼允許重複執行？去重需要中心排程器——這正是本設計去掉的東西。代價是偶爾多跑一次（冪等的）建置；收益是任何機器都能貢獻 CI 算力，且一切可稽核。",
  "guide.bugs.title": "看起來像 bug——其實不是",
  "guide.bugs.lede": "去中心化把一致性擺到明面上，而不是藏起來。你會遇到的四種景象：",
  "guide.bug1.sight": "同一個任務跑了兩次",
  "guide.bug1.truth": "合法競態。兩個 runner 在同一窗口認領；winner 規則選出唯一生效結果，另一條保留供稽核。",
  "guide.bug2.sight": "任務被認領了卻遲遲沒結果",
  "guide.bug2.truth": "runner 可能掛了。認領會過期（TTL）；另一個 runner 會重新認領同一次嘗試——不會遺失。",
  "guide.bug3.sight": "卡片在看板上的位置跳了",
  "guide.bug3.truth": "排序是（最後活動，id）。同一秒內的兩次活動按 id 排序——確定性規則，不是渲染故障。",
  "guide.bug4.sight": "結果「消失」了",
  "guide.bug4.truth": "它只是不生效。每條條目都還在 refs 上；存在不可撤銷，生效是規則的選擇。",
  "guide.cmds.title": "不確定時——三條指令",
  "guide.cmd1.what": "重放任意討論串：每條條目都帶著簽名和父鏈",
  "guide.cmd2.what": "離線算出的看板——與本頁位元組一致",
  "guide.cmd3.what": "全域概覽：討論串、驗證情況、活躍度、CI 小節",
  "guide.protocol": "規範性規則見 docs/D1_CI_PROTOCOL.md 與 AGENTS.md §2。",
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
