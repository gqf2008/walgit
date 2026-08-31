# D1 — 去中心化协作层设计（Decentralized collaboration on walgit）

> 状态：**设计提案**（design proposal），供讨论与立项，尚未实现。
> 本文描述一个**构建在 walgit 之上**的外部协作层——它不是 walgit 进程内的功能。文中出现的
> API / 路径 / 命令均为**提案**，未实现前不视为存在（与 `RULE_技能文档命令须有可执行实现` 对齐：
> 文档声明的能力必须由实现兑现，二选一）。
> 与 walgit 的关系遵循 `AGENTS.md §3` 原则 X（keep walgit small）与 `GOAL.md §4`：
> code review / merge queues / CI / issues 不在 walgit 范围内，**build on this**。

## 1. 目标

1. **仓库在 S3，唯一事实源是桶**。git 数据由 walgit 托管（WAL + manifest CAS，实例无状态、可随时抹掉），
   本地盘不承载仓库；协作层同样不留权威状态在实例上。
2. **协作层去中心化（D1）**：没有"协作服务器"拥有 issue/PR/评审数据。协作状态是**签名、追加式的
   git 对象**，存放在仓库的 `refs/collab/*`，由 walgit 顺手托管；任何人都能克隆、验签、回放、离线计算视图。
3. **一个 token 无缝协作**：人类和 agent 各持一个凭据（token ↔ principal ↔ 签名公钥），即可完成
   clone/fetch/push、协作读写、dashboard 观察，无需在多个系统分别开户。
4. **agent 与人对等**：agent 是一等参与者，同一协议、同一身份模型、同一权威规则；默认"agent 提议、
   人拍板"，策略可配。
5. **dashboard 只读可观测**：视图是任何客户端都能确定性算出的纯函数；dashboard 只是其中一种渲染端，
   不持有状态、无写权限。

## 2. 非目标

- 不做 **D2**（跨实例/跨组织联邦）与 **D3**（纯 P2P、无共享桶）。本设计的模型是
  **共享事实源（桶）+ 无中心协作服务器**。
- 不重造 git 内部（对象格式、pack、传输协议）；只消费 walgit 已有的能力。
- 不把协作逻辑塞进 walgit 进程（保持 walgit 小、专注；协作层是外部协议 + 客户端 + 可选薄服务）。

## 3. 架构总览

```
                 S3 / GCS bucket（唯一事实源：WAL + packs + collab refs）
                                   ▲
        读写都经 walgit（serve / maintain / events 角色）
        ┌──────────────────────────┴───────────────────────────┐
        │                    walgit（smart HTTP）               │
        │   receive-pack（push 唯一写口） / upload-pack / API    │
        │   events 桥：ref 事件 webhook（at-least-once, 可回放）   │
        └───────┬───────────────────┬─────────────────┬────────┘
                │                   │                 │
        ┌───────▼──────┐   ┌───────▼──────┐   ┌───────▼───────┐
        │ 人类客户端     │   │ agent 运行时   │   │ dashboard     │
        │ web UI/CLI/IDE│   │ 评审/写码 agent │   │ 只读可观测     │
        └───────────────┘   └──────────────┘   └───────────────┘
        全部是对等参与者：读同一份 refs、本地验签、确定性聚合
```

关键数据流：

- **写协作**：客户端构造签名条目 → `git push` walgit 的 `refs/collab/inbox/<principal>/<uuid>`
  → walgit 校验 policy + manifest CAS 入 WAL → 全局可见。
- **读协作**：refs 级同步（O(1)，无 pack）→ 本地验签 → 确定性聚合出线程/PR 视图。
- **git 事件 → 协作**：push 进 WAL → events 桥发 ref 事件（去重键 `(repo, seq, ref)`，可回放）
  → agent / CI / dashboard 订阅。
- **合并**：聚合视图判定规则满足（如 ≥1 个人类 approve）→ 合并方 `git push` 结果到 walgit
  → `policy.json` 兜底。

## 4. 协作数据模型

### 4.1 refs 命名空间（每仓库）

| ref | 内容 | 写权限 |
|---|---|---|
| `refs/collab/inbox/<principal>/<uuid>` | 某参与者的一条追加式条目链（签名字节） | 仅该 principal（policy 锁） |
| `refs/collab/meta/principals` | 注册表：principal → Ed25519 公钥（含吊销标记） | 仅 token 签发方 / 首次注册 |
| `refs/collab/meta/rules` | 合并规则对象（协作层语义，见 §6） | 仅 admin |
| `refs/collab/meta/protocol` | 协议版本号 | 仅 admin |

设计要点：

- **收件箱模型**：每人只写自己的 ref → 无跨参与者写冲突；同一收件箱内并发由 walgit 的
  manifest CAS（412 重试）保证。这与 walgit"多写者正确性由 CAS 保证"是同一哲学。
- **视图不存储**：issue 线程、PR 状态、评审统计都是**确定性纯函数**，输入 =
  (协议版本, 全部收件箱 refs 在某 manifest seq 的快照)。人人算出一致结果 → 不需要中心索引，
  没有中心服务可依赖/可单点故障。
- **跨 ref 一致性**：一次读取以单个 manifest CAS 为同步点（walgit 的 refs 在同一 manifest 下本就一致），
  避免读到中途的中间态。

### 4.2 条目（entry）schema

条目是内容寻址的 git 对象（blob），链式组织：

```json
{
  "version": 1,
  "kind": "issue | comment | patch | review | status | merge_result | agent_action",
  "id": "<uuid>",
  "actor": "alice@example.com",
  "ts": 1786500000,
  "parent": "<上一条目 oid 或根>",
  "refs": { "base": "refs/heads/main", "head": "refs/heads/topic" },
  "body": { "...": "..." },
  "sig": "ed25519:<base64>"
}
```

- **验签**：本地用 `refs/collab/meta/principals` 里该 actor 的公钥验 `sig`；
  链式 `parent` 保证追加顺序不可篡改（防重排、防丢）。
- **条目类型语义**：
  - `issue`：开 issue。
  - `comment`：线程评论（可带 file/line 锚点 → 评审行内评论）。
  - `patch`：提 PR（含 base/head refs、title/body）。
  - `review`：评审结论 `approve | request_changes | comment`（可带锚点）。
  - `status`：状态流转（in-progress / needs-review / blocked / needs-human …）。
  - `merge_result`：合并结果（result oid、规则求值记录）——由合并方写入，审计可回放。
  - `agent_action`：agent 行为留痕（可选；模型、置信度、耗时等元数据，供 dashboard 与审计）。

### 4.3 确定性聚合（视图即纯函数）

- `thread(id)` = 按 (parent, ts) 排序的、引用该 id 的条目序列。
- `pr(id)` = {base, head, 状态机（open/merged/closed）, reviews[], approvals[], 规则求值}。
- `merge_rule_eval(rule, log)` → `{allowed, reason, satisfied_by[]}`。

聚合不落库、不以缓存为权威（可做只读缓存加速热路径，权威永远是 refs）。

## 5. 身份与凭据：一个 token 走天下

目标：人类和 agent 只需要一个凭据即可无缝协作（git + 协作 + dashboard）。

- **token = 认证**：walgit 的静态 token / `wgt_` / OIDC，认证 git smart HTTP、API、协作写。
- **principal = 身份**：token 解析为 principal（人 `alice@…` 或 agent `svc:reviewer-1`）。
- **公钥 = 验签**：Ed25519 密钥对与 principal 绑定，注册进 `refs/collab/meta/principals`。
  **建议**：token 签发时一并生成/注册密钥对（扩展 `wgt_` 签发流程为"principal 注册"一步），
  这样"一个 token"同时覆盖认证与去中心化验签。
- **读可以直连桶（可选强化）**：bundle 走 presigned URL、静态对象走 S3 读——有凭据即可；
  **写永远走 walgit receive-pack**（manifest CAS 是唯一提交点，原则 II），
  所以"一个 S3 token"不意味着绕过 walgit 写。
- **agent 与人类完全同构**：agent 也是一份 token + 密钥对 + principal（`svc:` 前缀）；
  无特权路径，只有 policy 和合并规则赋予的权限。

## 6. 权威与合并规则

- **git 层**：`policy.json` 保护 `refs/collab/*`（收件箱只允许本人写；meta 只允许 admin），
  以及受保护分支（沿用现有 `match.refs` + `protect` + `bypass` 机制）。
- **协作层**：合并规则 = 确定性函数（`merge_rule_eval`），例如受保护分支要求
  `≥1 个人类 approve 签名`（agent 的 approve 默认不计入人类门禁，除非显式配置）。
  规则对象存于 `refs/collab/meta/rules`（协作层语义，不塞进 `policy.json`——
  与 `docs/POLICY.md` 的边界一致：required reviews / must go through a PR 属于合并规则，不属于 push 文件）。
- **人在回路默认**：受保护分支，agent 只能 propose + review；合并由人（或显式配置的
  merge-queue 服务）执行。非保护分支/个人仓库按配置放开。
- **审计**：一切动作（含合并）都是签名条目或 WAL 条目，可回放到任意点。

## 7. Agent 协议

1. **触发**：订阅 walgit events 桥的 ref 事件（`(repo, seq, ref)` 去重、at-least-once、可回放），
   或对 `refs/collab/*` 做 refs 级轮询（便宜，O(1)，无 pack）。
2. **行动**：构造签名条目（review / patch / comment / status）→ push 自己的收件箱。
   幂等：条目内容寻址 + `(id, kind, actor, parent)` 去重；事件按 seq 去重。
3. **上下文**：walgit API（tree/blob/commits/resolve）+ blobless bundle（全量上下文、字节走桶）。
4. **求助**：`status: needs-human` 条目 + `review: request_changes` 把球踢回人；dashboard/订阅者展示。
5. **留痕**：`agent_action` 条目记录模型/置信度/耗时（可选），供 dashboard 与审计。

## 8. Dashboard / 可观测性

- **只读、无状态、无写权限**：它只是"确定性聚合 + walgit API/指标"的一个渲染端。
- **视图**：issue/PR 线程、评审状态、approve 覆盖、agent 活动流、合并统计、
  事件滞后（`head_seq − cursor`）。
- **指标来源**：walgit `/metrics/prometheus`、`/healthz`、`/readyz`、`/api/*`（SSE 实时流），
  加上聚合视图。
- **实时**：订阅 events 桥 / SSE 信封，dashboard 与数据同步无轮询竞态。

## 9. walgit 缺口清单（需要补的能力）

| # | 缺口 | 说明 | 契合点 |
|---|---|---|---|
| 1 | **通用 refs 读取 API** | API 现只有 `refs/{branches|tags}`；补 `refs/collab/*` 的列出/读取（git 协议层已通告任意 ref） | 纯读、可缓存 |
| 2 | **评审原语端点** | `diff`、`merge-base`、`patch`、`blame`、`archive`（PR review 的 UI 和 agent 都要） | 纯函数、immutable、可缓存，符合 API 缓存规则 |
| 3 | **token↔公钥注册** | 签发 `wgt_` 时可选注册 Ed25519 公钥到 principals 注册表（或由协作层负责） | "一个 token 走天下"的前提 |
| 4 | **events 桥对任意 ref** | 验证 collab refs 的 ref 事件完整送达（理论上是，需 golden 测试） | 复用现有桥 |
| 5 | **SDK 扩展** | `repos.js` 增加 collab lane（读聚合、写条目） | dogfood 规则：不另起 SDK |

> 进度：缺口 1 由 [issue #8](https://github.com/gqf2008/walgit/issues/8) 批次落地——
> 已实现 `GET /{o}/{r}/api/refs/all`（全量 ref，全名、字节序分页、SSE）、
> `GET /{o}/{r}/api/refs/collab`（`refs/collab/*` 命名空间）与
> `GET /{o}/{r}/api/refs/name/{rest}`（精确单 ref 读取，SWR+ETag）。
> 缺口 2 进展：`GET /{o}/{r}/api/merge-base?from=&to=` 已落地（本地 `git merge-base`；
> remote 走有界双向 walk，含 SSE 叙述与预算保护）；
> `GET /{o}/{r}/api/diff?from=&to=&format=patch|stat|name-status` 已落地
> （remote 先 level-parallel fault 两树差异对象再跑同一 `git diff`，与本地字节一致）；
> patch（format=patch 即完整 unified diff）/blame/archive 待办。

## 10. 一致性、并发与安全

- **写并发**：收件箱按 principal 分片 → 无跨参与者竞争；同收件箱内 CAS 重试。
- **读一致性**：以单次 manifest CAS 为同步点读取全部 collab refs。
- **防滥用**：条目大小上限、频率配额（policy/代理层）、公钥吊销（principals 注册表支持 revoke 条目）。
- **隐私**：collab refs 与仓库同权限域——匿名/私有仓库的协作数据随之同权限（walgit 认证已覆盖）；
  独立的可见性控制（如 issue 对组织外可见）留作后续，本设计不引入。

## 11. 开放问题 / 下一步

1. 协作写走"裸 `git push` 收件箱"还是"薄 API 包装 push"？（倾向先裸 push，零新服务；API 包装后加）
2. Web UI 先行还是 CLI 先行？（建议 CLI + 最小 Web 视图先跑通协议）
3. 聚合视图的只读缓存放哪（是否复用 walgit 的 render cache `cache/api/v1/*.json`）？
4. 条目 GC/压缩：追加式长期膨胀，可做 checkpoint（聚合状态快照）——借鉴 walgit checkpoint 思路，设计期留 TODO。
5. 原型顺序建议：
   ① walgit 侧：通用 refs API + 评审原语端点；
   ② 协作条目协议 + CLI 验证（先于 UI）；
   ③ agent 接入（events 订阅 + 签名条目）；
   ④ dashboard（只读渲染端）。
