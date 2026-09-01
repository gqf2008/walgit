# D1-CI — 去中心化 CI 协议：事件触发、客户端算力、签名结果回传

> 状态：**规范**（normative）。本文是批次 issue #31 的设计文档：触发 / 认领 / 结果三类对象的
> schema、运行状态机、竞争收敛规则、TTL 重认领、去重、失败重试、产物引用与秘密边界，
> 全部写成可实现的规范。可执行形式：`crates/walgit-wal/src/ci.rs`（聚合核心）与
> `crates/walgit-cli/src/ci_cmd.rs`（`walgit ci validate|run|status`），测试即其黄金用例
> （与 `docs/POLICY.md`、`docs/EVENTS.md` 同一纪律）。
>
> 与 walgit 的关系：**服务端零 CI 逻辑**（原则 X）。walgit 只提供两样东西——事实源（桶：
> 代码对象与 `refs/collab/*`）与事件源（ref 事实，D32 events 桥是其 push 形态）。
> 跑任务的算力来自**持有凭据的客户端 runner**（人的机器 / agent 的机器），它以普通
> git 客户端的身份 fetch 代码、认领、执行、签名回传结果。walgit 进程内没有任何
> "CI"代码；`GOAL.md §4` 的边界不变。

## 1. 定位与不变式

1. **无中心调度器**：没有服务、队列或数据库决定"谁跑这个任务"。一切协调状态都是
   `refs/collab/inbox/<principal>/<uuid>` 里的**签名条目**（D1 §4.2），任何客户端都能
   离线验签、回放、重算出同一个答案。
2. **认领不是 walgit lease**（与 D7 的边界）：D7 租约是服务端维护回路（compact/bundle）
   的跨实例互斥，存桶内 `leases/*.pb`，由 manifest CAS 保证。CI 认领是协作层条目，
   收敛靠**对条目日志的确定性规则**，不靠互斥——两个 runner 可以同时执行同一任务
   （互不知晓），协议保证的收敛点在**结果**：生效结果恰有一份。这是用幂等与收敛替代
   互斥，去中心化系统的正确取舍。
3. **任务声明随代码走**：`.walgit/ci.toml` 是被测提交里的文件，被测哪个提交就读哪个
   提交的声明——没有中心注册表，没有"配置漂移"。
4. **一切都是可验证的**：claim 与 result 都经 Ed25519 签名、进签名者的收件箱、受
   `policy.json` 与 D1 §4.1 收件箱归属不变量约束。验签失败 = 红（不参与收敛计数）。
5. **秘密只在客户端**：环境变量属于 runner 进程；桶、ref、结果对象里没有任何秘密值
   （§9 是可机器检验的规范）。

## 2. 参与者与身份

- **runner 是一个普通 principal**（D1 §5）：一份 token（git 认证）+ 一对 Ed25519 密钥
  + 一个 principal（惯例前缀 `ci-` 或 `svc-`，如 `ci-runner-1`；语法与校验同 D1）。
  首次使用自注册进 `refs/collab/meta/principals/<principal>`。
- 写路径与人类/agent 完全同构：签名条目 → 自己的收件箱 ref → receive-pack。仓库若用
  `policy.json` 保护 `refs/collab/*`（D1 §6），CI 条目同样受其约束——CI 没有特权路径。
- 同一仓库可以并存任意多个 runner；同一 principal 也可以有多个 runner 进程（认领以
  principal 为身份，见 §6.4 崩溃恢复）。

## 3. 任务声明：`.walgit/ci.toml`

随代码版本化的 TOML 文件，位于被测提交树内的 `.walgit/ci.toml`。

```toml
version = 1                # 必填，当前必须为 1

claim_ttl  = "5m"          # 管道级认领 TTL（默认 "5m"；字符串 humantime）
timeout    = "10m"         # 管道级任务超时（默认 "10m"）
max_attempts = 1           # 失败自动重试上限（默认 1 = 不重试）

[[task]]
name     = "test"                          # 必填；[A-Za-z0-9._-]{1,64}；全文件唯一
refs     = ["refs/heads/*", "refs/tags/v*"] # 触发 ref glob（默认 ["refs/heads/*"]）
command  = "cargo test --quiet"            # 必填；经 shell 执行（≤ 4096 字节）
timeout  = "30m"                           # 覆盖管道级
max_attempts = 2                           # 覆盖管道级
env_allow = ["RUSTFLAGS", "CARGO_NET_GIT_FETCH_WITH_CLI"]  # 透传给任务的环境变量名（§9）
```

### 3.1 校验规则（normative，`walgit ci validate` 逐条实现）

| # | 规则 | 违反 = 校验失败 |
|---|---|---|
| V1 | 未知键拒绝（serde `deny_unknown_fields`，含 `[[task]]` 内） | 拼错的键直接红 |
| V2 | `version` 存在且 `== 1` | — |
| V3 | `task` 数量 ∈ [1, 64]；`name` 唯一且匹配 `[A-Za-z0-9._-]{1,64}` | — |
| V4 | `command` 非空，UTF-8 ≤ 4096 字节 | — |
| V5 | `refs` 每条 ≤ 64 个、长度 ≤ 255、必须以 `refs/` 开头；省缺 = `["refs/heads/*"]` | — |
| V6 | `timeout` ∈ [1s, 24h]；`claim_ttl` ∈ [1s, 24h]；humantime 解析失败即红 | — |
| V7 | `max_attempts` ∈ [1, 10] | — |
| V8 | `env_allow` 每项匹配 `[A-Za-z_][A-Za-z0-9_]*`，≤ 64 项；**禁止 `WALGIT_CI_*`**（runner 注入保留名，§8.2） | — |
| V9 | 文件本身 ≤ 64 KiB | — |

### 3.2 触发匹配语义

- 事件 = **一个 ref 的 tip 移动**（§4）。对被触发 ref `R` 的当前 tip 提交 `C`：读 `C`
  树内的 `.walgit/ci.toml`（`git show C:.walgit/ci.toml`；不存在 = 该 ref 无任务），
  对每个 `task` 求值 `refs` glob 是否匹配 `R`——匹配则 `(R, C, task)` 是一次**待运行**。
- glob 语义（normative，与 git refspec 一致的宽松子集）：`*` 匹配任意字符**含 `/`**，
  `?` 匹配单个字符，其余字面匹配，必须匹配整个 ref 名。`refs/heads/*` 命中
  `refs/heads/a/b`。
- runner 只考察 `refs/heads/*` 与 `refs/tags/*` 两类 ref（其余命名空间——`refs/collab/*`、
  `refs/pull/*` 等——不是触发面）。
- **声明的权威版本 = 被测提交里的版本**：ref `R` 的 tip 是 `C`，就用 `C` 的 ci.toml 决定
  跑什么。其他 ref 的 ci.toml 对 `R` 无发言权。

## 4. 触发（trigger）：ref 事实与两种传输

**触发面是事实，不是消息**："`ref R 的 tip 变成了 X`"。事实由 WAL 的 PUSH/REF_UPDATE
条目产生（D32），消费端可以从两条传输观察到同一事实：

1. **refs 级轮询（normative 默认，本批次实现）**：`git ls-remote` 每 `interval` 秒一次
   （一个往返，无 pack），与本地状态文件对比得"变化过的 ref"。对离线一段时间后回来的
   runner，对比自然**合并（coalesce）**为"处理当前 tip"——中间的多次推送折叠成最后一次。
   这与 events 桥的 backfill 契约同一哲学：正确性不依赖推送，只依赖事实。
2. **events 桥 webhook（push 形态，扩展点，本批次未实现）**：常驻托管 runner 可挂
   `events.webhook_url`，按 `docs/EVENTS.md` 验签、按 `X-Walgit-Delivery` 去重、按
   `(repo, seq, ref_name)` 去重，把每条 ref 事件当作一次触发提示。实现它不改变本协议的
   任何对象或状态机——它只是把 §5 步骤 1 的输入从轮询换成推送。

**状态与去重（runner 侧）**：状态文件 `<gitdir>/ci-run.json` 记录 `{"processed": {ref: oid}}`。
一次 pass 里：当前 tip ≠ 已处理 oid 的 ref 是**待处理**；对该 ref 的**全部**任务到达终态
（结果生效或任务被跳过）才写入 processed。未到终态（让位、等待他人认领）则**不写**——
下个 pass 重估（这使 TTL 过期重认领无需任何额外机制，§6.3）。

## 5. 运行（run）与运行标识

- 一次 run = `(task, ref, commit)` 的一个执行单元；同一三元组多次重试是同一 run 的多个
  **attempt**（§7）。
- **run id（normative，任何客户端可独立复算）**：

  ```
  run_id = "ci-" + hex16(fnv1a64( utf8(task) || 0x1f || utf8(ref) || 0x1f || utf8(commit) ))
  ```

  `fnv1a64` 为 64 位 FNV-1a；`hex16` 为其大端 64 位值的 16 个小写 hex 字符。短、URL
  安全、确定性、无碰撞现实风险（2^32 次运行才到生日界）；可读字段（task/ref/commit）
  在条目 body 里。run id 就是协作线程 id（D1 §4.2 `id`），claim 与 result 因此落在同一
  线程，`walgit collab thread <run_id>` 直接可看。

## 6. 认领（claim）：`ci_claim`

### 6.1 条目 schema（normative）

`kind = "ci_claim"`，`id = run_id`，`actor = runner principal`，`ts = 认领时刻（entry 的
签名字段，收敛用它）`，`parent = ""`（认领不是对前一条目的回复，而是对运行的新一轮
竞争；多个认领 = 同一线程的多个根，线程排序按 §4.3 的 (ts, actor, oid)）：

```json
{
  "task": "test",
  "ref": "refs/heads/main",
  "commit": "cb38da1…",
  "ttl": 300,
  "attempt": 1,
  "runner": "walgit ci/0.1"
}
```

`ttl` 单位秒（必填，∈ [1, 86400]）；`runner` 信息性可选。其余字段必填、类型严格；
类型不符的条目视为 **malformed**：计入展示但不参与收敛（与验签失败同待遇）。

### 6.2 认领算法（runner 的一次 pass 内，对每个待运行）

1. **读**：fetch `refs/collab/*`，聚合出该 run 的当前视图（§7 的 `run_view`，含 `now`）。
2. **决策**（`decide()`，normative 见 §7.3）：
   - `Settled` → 跳过（写 processed）；
   - `StandDown{reason}` → 跳过（**不**写 processed，§4）；
   - `Resume{claim_oid}`（胜者是我自己的未过期认领——进程重启/崩溃恢复）→ 直接执行；
   - `Claim{attempt}` → 签名发布 claim 条目（push 收件箱）。
3. **复核（convergence point）**：claim push 成功后**重新 fetch** `refs/collab/*`，重算
   视图。若胜者仍是我的 claim → 执行；否则 → **让位**（stand down，不执行、不写
   processed）。竞争窗口 = 两个 push 之间，收敛点 = 确定性胜者规则，两者缺一不可：
   没有复核，两个同时"先读后写"的 runner 会都执行；没有确定性规则，复核后无法裁决。

### 6.3 TTL 与重认领（normative）

- 认领在 `ts + ttl` 时刻过期（`ts` 是条目自己的签名字段）。过期认领不再参与胜者计算
  （`decide` 给出 `Claim`，允许任何人重新认领）。
- **过期重认领 = 发布一条新 claim**（新的 ts、新的 uuid ref）。旧 claim 留在日志里
  （追加式，不删不改）——它解释了"为什么前一次认领没有结果"。
- runner 崩溃 → 没有 result → 认领过期 → 任何 runner（含崩溃者自己重启后）重新认领。
  没有僵尸锁、没有清除进程，过期即自愈。
- `conclusion = "error"` 的结果（§8 的基础设施失败）**作废其引用的 claim**：该 claim 视同
  已过期，允许立即重认领（同 attempt），不必等 TTL——执行方自己报告了"我没跑成"。

### 6.4 时钟偏斜

收敛是**条目集合 + 评估者 `now`** 的纯函数。各 runner 时钟有偏斜时：胜者（谁执行）
在 TTL 边界附近可能随评估者而异——这影响的是**活性**（可能多跑或少跑一次），不是
**收敛性**：已生效的结果不随 `now` 或视角改变（§7.2 的 fallback 规则保证 settled 保持
settled）。协议不做时钟同步；偏斜大的部署把 `claim_ttl` 调大即可。

## 7. 聚合与状态机（`walgit-wal::ci`，normative）

### 7.1 输入与纯函数

```text
run_view(entries, principals, now) -> RunView      // 按 id 分组后的单 run 视图
collect_runs(entries, principals, now) -> BTreeMap<run_id, RunView>
decide(run, actor, max_attempts) -> Decision      // §6.2 步骤 2 的规范实现
                                                  //（评估时刻已由 run_view(.., now) 携带）
```

只计入**已验证**条目（`EntryRef::is_verified`：验签通过 **且** 收件箱归属正确，D1 §4.1）；
未验证/malformed 条目计入展示计数（红），不参与状态。

### 7.2 胜者与生效结果（收敛规则，normative）

对一次 run 的某个 attempt（同一 `id` + `attempt` 值）：

```text
claims  = 已验证 ci_claim，按 body 解析成功者
results = 已验证 ci_result，其 body.claim 指向 claims 中某个 oid 者

valid(c)   = c.ts + c.ttl > now  且  c 未被 conclusion=="error" 的结果引用   // §6.3
winner     = min by (ts, actor, oid) over { c ∈ claims | valid(c) }         // 最早者胜
effective  = min by (ts, actor, oid) over { r ∈ results | r.claim == winner.oid }
           若 winner 不存在（全部过期/作废）但 results 非空：
           effective = min by (ts, actor, oid) over results                  // fallback
```

- **恰一份生效结果**：胜者全序唯一 ⇒ `effective` 全序唯一。两个 runner 同时执行并各自
  发结果（互不知晓的分区情形）时，引用非胜者 claim 的结果**记录在案但不生效**——
  看板与状态机只认 `effective`。
- **fallback 保证 settled 不回退**：生效结果的 claim 过期后，`winner` 变空，fallback 仍
  指向同一结果——done 不会因 TTL 流逝而退回 pending，重认领也不会把已成功的结果挤掉。
- 重认领后的新结果引用新 claim：新 claim 是唯一 valid 胜者 ⇒ 新结果生效，过期认领者
  迟到的旧结果不生效（§6.3 的竞争场景）。

### 7.3 状态机与 `decide()`

attempt 状态（纯函数，`now` 为参数）：

```text
pending : claims 为空                                  → 任何人可 Claim
claimed : winner 存在，无 effective                    → 胜者 Resume，其他人 StandDown
stale   : claims 非空但无 valid winner（TTL 过期或被 error 作废）→ 任何人可 Claim（重认领）
done    : effective 存在                               → Settled(conclusion)
          conclusion ∈ {success, failure} 终态；
          conclusion == "error" 时 effective 仍展示，但其 claim 已作废 ⇒ 状态实为 stale（§6.3）
```

`decide(run, actor, max_attempts)`：

```text
最新 attempt n* = run 内出现过的最大 attempt（无则 0）
若 attempt n* 是 done(success)                 → Settled
若 attempt n* 是 done(failure):
      若 actor 决定重试且 n*+1 ≤ max_attempts → Claim{attempt: n*+1}   // §7.4
      否则                                     → Settled(failure)
若 attempt n* ∈ {pending, stale}               → Claim{attempt: n* (无则 1)}
若 attempt n* 是 claimed 且 winner.actor==actor → Resume{winner}
若 attempt n* 是 claimed 且 winner 是别人       → StandDown{who}
```

`max_attempts` 的语义：**是上限不是配额**——重试由 runner（通常是刚失败的自己）在
`decide` 给出 `Claim{attempt: n+1}` 时发起；聚合核心只呈现事实，不读 ci.toml（读侧不
依赖被测树，保持纯函数）。是否真重试是 runner 策略：本批次的 runner 对
`done(failure)` 且 `attempt < max_attempts` 立即重试一次。

### 7.4 失败重试（normative）

- attempt 从 1 起连续编号；每次重试 = 新 claim（`attempt: n+1`）+ 新结果，同一线程。
- 重试触发条件：上一 attempt `done(failure)`。`success` 不重试；`timeout` 按 failure 处理
  （可重试）；`error` 不增加 attempt（同 attempt 重认领，§6.3）。
- 上限 `max_attempts`（ci.toml §3）在 runner 侧执行；超过即 `Settled(failure)`。
- 幂等：执行到一半崩溃（结果未发）→ claim 过期 → 重认领重跑整个任务。**at-least-once
  执行、exactly-once 生效结果**——任务必须自身容忍重复执行（CI 任务天然如此）。

## 8. 执行与结果：`ci_result`

### 8.1 执行（runner，normative）

- **工作区**：`git worktree add --detach <tmp> <commit>` 于 runner 自己的检出内；任务在
  该目录执行；结束后 `worktree remove --force` + `prune`（失败不阻塞结果回传）。
- **命令**：`sh -c <command>`（POSIX）；Windows 上 `cmd /C <command>`。命令的 cwd =
  工作区。
- **环境（秘密边界，§9）**：`env_clear()` 后只给：平台基础集（`PATH`, `HOME`, `TMPDIR`,
  `TEMP`, `TMP`, `SYSTEMROOT`, `SYSTEMDRIVE`, `COMSPEC`, `PATHEXT`, `USERPROFILE`，
  取 runner 自身环境值）∪ ci.toml `env_allow` 里声明的变量名 ∪ runner 注入的
  `WALGIT_CI_*`。
- **注入的任务变量**（保留名，V8 禁止 ci.toml 声明）：`WALGIT_CI_TASK`、`WALGIT_CI_REF`、
  `WALGIT_CI_COMMIT`、`WALGIT_CI_RUN_ID`、`WALGIT_CI_ATTEMPT`、`WALGIT_CI_ACTOR`。
- **超时**：到 `timeout` 杀进程（含子进程尽力而为），`conclusion = "timeout"`。
- **结论映射**：exit 0 → `success`；exit ≠ 0 → `failure`；超时 → `timeout`；runner 自身
  故障（fetch 失败、无法 spawn、工作区无法创建）→ `error`（不产出任务结论）。

### 8.2 结果条目 schema（normative）

`kind = "ci_result"`，`id = run_id`，`parent = <其 claim 的条目 oid>`（结果挂在认领之下，
线程时间线即认领→结果成对出现）：

```json
{
  "task": "test",
  "ref": "refs/heads/main",
  "commit": "cb38da1…",
  "attempt": 1,
  "claim": "<claim 条目 oid>",
  "conclusion": "success",
  "exit_code": 0,
  "duration_ms": 1234,
  "log_summary": "…输出末尾 ≤ 4096 字节…",
  "log_sha256": "<完整捕获输出的 sha256 hex；无输出为空串>",
  "artifacts": [
    { "name": "coverage", "path": "target/coverage", "sha256": "<hex>", "bytes": 12345,
      "url": "https://…" }
  ]
}
```

- `conclusion` ∈ `success | failure | timeout | error`；`exit_code` 为整数，`timeout`/无法
  取得时为 `null`。
- **日志摘要**：`log_summary` 是完整捕获输出（stdout+stderr 合并）的**末尾** ≤ 4096 字节
  （写入侧截断，UTF-8 字符边界对齐）；完整日志的完整性由 `log_sha256` 兜底——runner 可
  把完整日志留本地或作为 artifact 上传，walgit 不存日志正文。
- **产物（artifacts）**：大产物**不进 git 对象**——只放引用 + 哈希：`name` ≤ 128 字节、
  `path` 任务工作区内的相对路径、`sha256`（完整性）、`bytes`、可选 `url`（任何取用方自行
  验哈希；walgit 不解释 url）。每个结果 ≤ 32 个 artifact。
- 其余字段必填、类型严格；malformed 待遇同 §6.1。

### 8.3 展示（读侧）

- `walgit-wal::ci` 输出 `RunView` / `CiReport`（§7）；`walgit ci status`（text/markdown/json）
  与 `walgit collab report` 的 CI 段、SPA 线程时间线的红绿徽标 + 日志摘要，全部消费
  同一聚合——三端同源，无第二套语义。
- 验签失败 / malformed 条目永远可见（计数为红），不被静默吞掉。

## 9. 秘密边界（normative，可机器检验）

1. **ci.toml 不存秘密值**：它是版本化代码；`env_allow` 只出现变量**名**（V8）。
2. **任务环境是白名单**（§8.1）：runner 环境里的其他一切（含秘密）**不**进入任务进程，
   除非显式 `env_allow`。声明即知情：仓库作者点名哪个变量可用。
3. **claim / result 对象只含 schema 字段**（§6.1/§8.2）：runner 代码路径不存在"顺手带上
   环境"的通道；`log_summary` 是任务输出的忠实摘要，任务自己打印什么是仓库作者的责任——
   协议边界是 **runner 不注入**，负向测试据此构造（命令消费秘密但不打印它，断言两份
   条目对象与线程 JSON 均不含秘密值）。
4. **验收测试**（`crates/walgit-cli/tests/ci_e2e.rs`）：runner 以含秘密的环境变量执行后，
   断言 claim blob、result blob、`collab thread`/`ci status` JSON 均不含该值；同时以命令
   内 `printenv` 成功证明白名单注入是真实生效的（守卫见过红，见 RULE_规则可执行性）。

## 10. 去重规则汇总（normative）

| 层 | 键 | 规则 |
|---|---|---|
| 触发（轮询） | `(ref, tip oid)` | 状态文件 processed；tip 相同不重触发；coalesce 到当前 tip |
| 触发（webhook，扩展点） | `(repo, seq, ref_name)` / `X-Walgit-Delivery` | 按 docs/EVENTS.md |
| claim | `(id, attempt, actor)` | 同 principal 的重复认领无害：胜者规则全序，min 唯一 |
| result | `(id, attempt)` | `effective` 全序唯一；重复结果记录但不生效 |
| 事件重放 | 条目 oid（内容寻址） | 重放/重投递产生同一 oid，聚合幂等 |

## 11. 安全与滥用

- 收件箱模型 + `policy.json` 与 D1 §6/§10 完全一致：CI 条目可被冻结（保护
  `refs/collab/*`），恶意 runner 可 tombstone 吊销（其后续条目全部验签失败 = 不参与收敛）。
- 竞争成本：认领竞争的最坏代价是重复执行（at-least-once），不是状态破坏；没有可被
  垄断的中心队列。恶意抢认领（超早 ts）只赢得执行权，输给后来者唯一途径是交不出
  合法结果——系统向"有结果"收敛。
- 条目大小受 §3/§8 上限约束（写入侧执行）；读侧对超大 body 按 malformed 计。

## 12. 本批次落地范围与开放项

- **已实现**：§3（`walgit ci validate`）、§5–§9（`walgit ci run`，轮询传输）、§7/§8.3
  （`walgit-wal::ci` + `walgit ci status` + `collab report` CI 段 + SPA 线程徽标）、
  e2e（`crates/walgit-cli/tests/ci_e2e.rs`：端到端验签回放、双 runner 竞争收敛、
  kill 后 TTL 重认领、秘密边界负向、超时结论）。
- **开放项**（不影响协议对象与状态机）：events 桥 webhook 传输（§4.2，托管 runner 场景）；
  完整日志/产物的标准化存放位置（现在是引用 + 哈希，放哪由部署定）；按 cron 的定时
  触发（现在只有 ref 触发；定时 = 对一个不动的 ref 周期性评估，留给后续批次）。
