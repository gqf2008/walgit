#!/bin/bash
# 构建 macOS 托盘应用 walgit-tray.app(LSUIElement,菜单栏 + Dock 双驻留),
# 并把部署骨架打包进 Resources(首次启动托盘自动落盘 ~/walgit)。
# 依赖:Xcode Command Line Tools(swiftc)+ 已构建的 walgit release 二进制。
# 用法:./build.sh [版本]   —— 版本默认取仓库最近 tag(v0.2.0 → 0.2.0)。
# 产物:~/Applications/walgit-tray.app
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd ../../.. && pwd)"

VERSION="${1:-$(git -C "$ROOT" describe --tags --abbrev=0 2>/dev/null || echo v0.0.0)}"
VERSION="${VERSION#v}"
case "$VERSION" in
    ''|*[!0-9A-Za-z.+-]*) echo "invalid version: $VERSION" >&2; exit 1 ;;
esac

WALGIT_BIN="${WALGIT_BIN:-$ROOT/target/release/walgit}"
[ -x "$WALGIT_BIN" ] || { echo "missing walgit binary: $WALGIT_BIN (WALGIT_BIN 可覆盖)" >&2; exit 1; }

APP="${TRAY_APP_DIR:-$HOME/Applications}/walgit-tray.app"
BIN_DIR="$APP/Contents/MacOS"
RES_DIR="$APP/Contents/Resources"

swiftc -O -swift-version 5 -framework AppKit walgit-tray.swift -o walgit-tray

rm -rf "$APP"
mkdir -p "$BIN_DIR" "$RES_DIR"
cp walgit-tray "$BIN_DIR/walgit-tray"
# 部署骨架:首次启动 bootstrap 从 bundle 落盘 ~/walgit(已存在则不覆盖)
cp "$WALGIT_BIN" "$RES_DIR/walgit"
cp run-walgit.sh walgit-ensure "$RES_DIR/"
cp walgit.toml.template "$RES_DIR/walgit.toml"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>com.walgit.tray</string>
    <key>CFBundleName</key><string>walgit-tray</string>
    <key>CFBundleExecutable</key><string>walgit-tray</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleIconFile</key><string>walgit</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Dock 图标(icns)——如无现成 icns,app 会以通用图标显示,不影响功能
if [ -f walgit.icns ]; then
    cp walgit.icns "$RES_DIR/walgit.icns"
fi

echo "built: $APP (version $VERSION, walgit from $WALGIT_BIN)"
echo "启动:open $APP   开机自启:系统设置 → 通用 → 登录项 → 添加本 app"
