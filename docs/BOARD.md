# Board（`.walgit/board.toml`）

看板不是状态，也不是带独立写路径的视图：它是线程集合按**声明式列定义**折叠出的确定性投影。
列定义随仓库版本化在 `.walgit/board.toml`（普通 git 文件，改动和其它代码一样走提交+评审）；
**移动卡片 = 追加一条签名 `status` 条目**，投影据此重新派生列。

## Schema（`version = 1`）

```toml
version = 1

[sort]
by = "ts"          # ts（默认，按最后活动 last_ts）| id（按线程 id）
direction = "desc" # desc（默认，新在前）| asc

[[column]]
name = "open"      # 必填，列名
kind = ""          # 可选：线程必须包含该 kind 的条目
status = "open"    # 可选：卡片有效状态必须等于它
merge = ""         # 可选："allowed" | "blocked"（对 patch 的合并规则判定；无 patch 不匹配）
unverified = false # 可选：true 时只收"至少含 1 条未验签条目"的卡片
```

规则：

1. **首列命中**：卡片进入**第一个**满足谓词的列；所有字段都空的一列是 catch-all。
2. **未命中不显示**：不匹配任何列的线程不在看板上——"说你想看到的，而不是拿到你没要求的"。
3. **`status` 语义**（`card_status`）：按线程条目顺序重放，最后一个匹配者生效——
   每条 `status` 条目以 `body.status` 设置；`merge_result` 带 `{"merged": true}` 置 `merged`；
   更晚的 `status` 可覆盖；默认 `open`。
4. **确定性**：同一份 refs 任何客户端算出字节一致的看板（build_board 纯函数）。

## 使用

- 预览未提交的列定义：`walgit collab board --board <file> --repo <checkout> [--format text|markdown|json]`
- 默认看板：仓库未定义时，按 well-known status（open/in-progress/needs-review/done/merged/closed/other）各一列 + 排序默认值。

## 示例

见仓库根 `.walgit/board.toml.example`（open / in-progress / needs-review / done 四列）。
