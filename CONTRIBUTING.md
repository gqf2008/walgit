# Contributing

动手前先读 [AGENTS.md](AGENTS.md) —— 它是这个仓库的宪法：约束、决策、工作规则都在里面，本文件只是路标。

## 环境

- Rust 工具链由 `rust-toolchain.toml` 锁定；还需 `protoc`、`git ≥ 2.46`；web UI 需要 `node 24 + pnpm`。
- 一键环境：`nix develop`（flake.nix），或按 README 的 Platforms 段落自备。
- Windows 原生可编译可测（NTFS 卷 + Developer Mode）；容器仍是推荐部署形态。

## 流程

1. 从 issue 开始：同类同机制 ≥3 条合并为批次 issue + checklist（模板已内置）。
2. 在 worktree/分支上开发，不在 main 直接改。
3. 自跑门禁：`just warnings && just clippy && just test`（当前 clippy/warnings 有预存债务，
   见跟踪 issue —— 增量必须为零）；smart HTTP 改动加 `just e2e`。
4. PR 按模板填写全部章节（Verification / Model Used / 审查）；重大改动必须有独立审查者。
5. CI 全绿 + 审查通过后合并，合并即清理分支。

## 提交

Conventional Commits（`feat|fix|chore|docs|refactor|test|perf(scope): 描述`），一个提交一个逻辑变更，
信息说"为什么"。

## 发布

- 版本语义化（semver）：`v<major>.<minor>.<patch>`，tag 触发 `release.yml`
  （构建 linux/windows 产物、从 Conventional Commits 生成 changelog、挂到 GitHub Release）。
- 发布本身是一个工作单元：开 `batch` issue 列发布清单（里程碑 `v0.1` 是首个目标）。
- 里程碑与批次的关系：一个里程碑一个版本，issue 挂里程碑表示"进这个版本"。
