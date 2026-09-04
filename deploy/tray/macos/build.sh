#!/bin/bash
# 构建 macOS 托盘应用 walgit-tray.app(LSUIElement,菜单栏 + Dock 双驻留)。
# 依赖:Xcode Command Line Tools(swiftc)。产物:~/Applications/walgit-tray.app
set -euo pipefail
cd "$(dirname "$0")"

APP=~/Applications/walgit-tray.app
BIN_DIR="$APP/Contents/MacOS"
RES_DIR="$APP/Contents/Resources"

swiftc -O -swift-version 5 -framework AppKit walgit-tray.swift -o walgit-tray

mkdir -p "$BIN_DIR" "$RES_DIR"
cp walgit-tray "$BIN_DIR/walgit-tray"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>com.walgit.tray</string>
    <key>CFBundleName</key><string>walgit-tray</string>
    <key>CFBundleExecutable</key><string>walgit-tray</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>1.1</string>
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

echo "built: $APP"
echo "启动:open $APP   开机自启:系统设置 → 通用 → 登录项 → 添加本 app"
