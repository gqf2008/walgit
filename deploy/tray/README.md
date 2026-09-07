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
- **打开 Web UI**:直接打开页面(三平台一致)
- **退出托盘(服务保持运行)**

## 升级语义

自动的只有「检测」:每 30 分钟(+启动 30 秒)`git fetch` 比对本地 main 与
origin/main,静默、失败不打扰。发现新版本 → 菜单行变「⬆️ 升级到新版本」+
系统通知;**升级必须由用户点击**。管线:ff-merge main → `cargo build
--release -p walgit-cli` → 备份(`walgit.bak-tray`)→ 停 → 热换 → 起 →
15s 健康验证,失败自动回滚。升级需要本机有 git 与 rustup/cargo。

## 约定

- 部署目录:`$HOME/walgit`(Windows:`%USERPROFILE%\walgit`),内含
  `walgit(.exe)` + `walgit.toml`;macOS 另需 `walgit-ensure` 与
  `run-walgit.sh`(release DMG 的托盘首次启动会自动落盘这四件骨架,
  已存在的文件不覆盖;凭证 `~/walgit/.r2-credentials` 由使用者自填)。
  macOS 首次启动还会幂等建 `/usr/local/bin/walgit` 软链 → 部署二进制,
  终端直接可用(测试用 `WALGIT_CLI_LINK` 覆盖)。
- **release 资产一个平台一件安装器(issue #108)**:
  macOS `walgit-<version>-arm64.dmg`(app 拖入 Applications,首次启动
  自动建部署骨架)、Windows `walgit-setup-<version>-x64.exe`
  (`deploy/windows/`,含托盘与本体)、Linux `walgit_<version>_amd64.deb`
  (`deploy/linux/build-deb.sh`,装 /usr/bin 三件 + 示例配置 + 托盘
  .desktop)。裸二进制不再发布。
- 服务地址:`http://127.0.0.1:8081`(托盘探活/开页与 `walgit.toml` 的
  `[server] listen` 同源解析,改端口不再需要改托盘;#73)
- 内存后端:托盘点状态行与 Web 概览页横幅显式标注「数据不落盘」(#73)
- 源码仓库:环境变量 `WALGIT_REPO`,默认 `/Volumes/Workspace/GitHub/walgit`
- 日志:`<部署目录>/tray.log`

## 构建

### macOS(Swift)

```bash
cargo build --release --bin walgit                 # 先有 walgit 二进制
cd deploy/tray/macos && ./build.sh 0.2.0           # 产物 ~/Applications/walgit-tray.app
./build-dmg.sh 0.2.0                               # 产物 dist/walgit-0.2.0-arm64.dmg
```

需要 Xcode Command Line Tools(swiftc)。`build.sh` 把 walgit 二进制 +
`run-walgit.sh` + `walgit-ensure` + `walgit.toml.template` 打进 app
Resources——首次启动 bootstrap 到 `~/walgit`(幂等)。`build-dmg.sh` 走全链:
app 签名+公证+装订(`~/scripts/notarize.sh`,profile `voicecall-notary`)
→ hdiutil 出 DMG(拖放安装)→ DMG 签名+公证+装订。Dock 品牌图标:把
`walgit.icns` 放在同目录再跑 build.sh(可选,缺省用通用图标)。开机自启:
系统设置 → 通用 → 登录项 → 添加 walgit-tray.app。

### Linux(.deb)

```bash
cargo build --release --bin walgit --bin walgit-server   # 先有二进制
cargo build --release --target-dir target --manifest-path deploy/tray/tray-rs/Cargo.toml
deploy/linux/build-deb.sh target/release 0.2.0           # 产物 walgit_0.2.0_amd64.deb
```

`dpkg-deb` 组装,零新依赖:三件二进制 → /usr/bin,`walgit.example.toml` 与
D43 未配置模板 → /usr/share/walgit,托盘 → /usr/share/applications;
postinst 为安装用户落盘 `~/walgit` 骨架(幂等不覆盖,与 mac DMG /
Windows 安装器一致)。CI 每个 PR 用 debug 二进制校验脚本(release.yml
打 tag 时用 release 二进制)。

### Windows / Linux / macOS(Rust)

```bash
cd deploy/tray/tray-rs && cargo build --release
# 产物 tray-rs/target/release/walgit-tray(.exe),放到部署目录运行即可
```

Linux 需要 `libgtk-3-dev libayatana-appindicator3-dev libxdo-dev`
(tray-icon 走 appindicator,默认 feature 引 libxdo)。Windows 上 release
产物为 GUI 子系统(无控制台)、单实例、首次运行自动创建部署目录并写
`tray.log`;细节见 `tray-rs/README.md` 的「Windows 说明」。
