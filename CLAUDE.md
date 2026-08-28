# CLAUDE.md

@AGENTS.md

读入上面这份架构与协作规则后遵守它；本文件只是入口指针，规则一律不改写在这里。
当前已知与规则不符的现实（agent 不要误判）：

- `clippy -D warnings` 工作区门禁存在预存债务（~1300 条，issue #1），CI 的 clippy
  job 已白名单化（known-red debt，不阻塞合并）；增量必须为零。
- e2e 与 sim 套件已在 Windows 上跑通（issue #2 已合并）；CI 的 windows leg 覆盖
  快速层 + sim + e2e。测试偶发红先查 AGENTS.md §5 known-flaky 名单与 §6.3 CI 信号。
