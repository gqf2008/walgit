# CLAUDE.md

@AGENTS.md

读入上面这份架构与协作规则后遵守它；本文件只是入口指针，规则一律不改写在这里。
当前已知与规则不符的现实（agent 不要误判）：

- `clippy -D warnings` 工作区门禁存在预存债务（~1300 条，见 fork issue 跟踪），CI 的
  clippy/warnings job 在清偿完成前预期为红；增量必须为零。
- e2e 与 sim 套件尚未在 Windows 上运行；Windows 快速层 = `just test` 的三段等价命令。
