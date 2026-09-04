# walgit 托盘应用(deploy/tray)

本机 walgit 服务的系统托盘:启停、新版本自动检测(检测到只提示,**由用户
点击才升级**)、退出仅退托盘(服务保持运行)。两个实现,语义一致:

| 目录 | 平台 | 技术 |
|---|---|---|
| `macos/` | macOS | Swift + AppKit(菜单栏 NSStatusItem + Dock 双驻留,彩色状态图标,品牌 Dock 图标,点击 Dock 聚焦已开 Web UI 标签) |
| `tray-rs/` | macOS / Windows / Linux | Rust + tray-icon + winit(独立 crate,**不加入** walgit workspace) |

## 菜单(两个实现一致)

- **状态行**:`walgit 服务:运行中 · <版本>`(5 秒轮询 /healthz)
- **启动 / 停止服务**:macOS 走 `walgit-ensure`(screen 保活);Windows/Linux
  分离进程启动部署目录下的 `walgit(.exe) serve`,pid 写 `walgit.pid`
- **版本升级状态行**(abb 式状态机):
  `版本 <sha> · 检查更新…` → `正在检查更新…` → `已是最新 ✓(点击重查)`
  → `⬆️ 升级到新版本 <新sha>(当前 <sha>)`(点击才升级)
  → `升级中… · 对齐 main/构建中/换装中` → 成功回「已是最新」/ 失败提示重查
- **打开 Web UI**:优先激活已开的 walgit 页面/窗口,绝不重复开页
- **退出托盘(服务保持运行)**

## 升级语义

自动的只有「检测」:每 30 分钟(+启动 30 秒)`git fetch` 比对本地 main 与
origin/main,静默、失败不打扰。发现新版本 → 菜单行变「⬆️ 升级到新版本」+
系统通知;**升级必须由用户点击**。管线:ff-merge main → `cargo build
--release -p walgit-cli` → 备份(`walgit.bak-tray`)→ 停 → 热换 → 起 →
15s 健康验证,失败自动回滚。升级需要本机有 git 与 rustup/cargo。

## 约定

- 部署目录:`$HOME/walgit`(Windows:`%USERPROFILE%\walgit`),内含
  `walgit(.exe)` + `walgit.toml`;macOS 另需 `walgit-ensure`(安装器/
  技能包提供)
- 服务地址:`http://127.0.0.1:8081`
- 源码仓库:环境变量 `WALGIT_REPO`,默认 `/Volumes/Workspace/GitHub/walgit`
- 日志:`<部署目录>/tray.log`

## 构建

### macOS(Swift)

```bash
cd deploy/tray/macos && ./build.sh     # 产物 ~/Applications/walgit-tray.app
```

需要 Xcode Command Line Tools(swiftc)。Dock 品牌图标:把 `walgit.icns`
放在同目录再跑 build.sh(可选,缺省用通用图标)。开机自启:系统设置 →
通用 → 登录项 → 添加 walgit-tray.app。

### Windows / Linux / macOS(Rust)

```bash
cd deploy/tray/tray-rs && cargo build --release
# 产物 target/release/walgit-tray(.exe),放到部署目录运行即可
```

Linux 需要 `libgtk-3-dev`(tray-icon 走 appindicator)。Windows/Linux 上
「聚焦已开页面」按窗口标题匹配(`walgit`),Linux 需 `wmctrl`、Windows 用
PowerShell AppActivate;失败时退化为直接打开页面。
