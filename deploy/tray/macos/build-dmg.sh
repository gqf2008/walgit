#!/bin/bash
# build-dmg.sh — 构建发布用 macOS DMG:walgit-tray.app + /Applications 拖放安装。
# 全程:app 签名+公证+装订 → hdiutil 出 DMG → DMG 签名+公证+装订。
# 依赖:build.sh(swiftc + target/release/walgit)、~/scripts/notarize.sh(app 公证)、
#       keychain 凭据(Developer ID 身份 + notarytool profile,默认 voicecall-notary)。
# 用法:./build-dmg.sh [版本]
# 产物:dist/walgit-<版本>-arm64.dmg
set -euo pipefail
cd "$(dirname "$0")"

VERSION="${1:-$(git -C "$(cd ../../.. && pwd)" describe --tags --abbrev=0 2>/dev/null || echo v0.0.0)}"
VERSION="${VERSION#v}"
PROFILE="${NOTARY_PROFILE:-voicecall-notary}"
DIST=dist
mkdir -p "$DIST"

# 1. app(含部署骨架资源)
TRAY_APP_DIR="${TRAY_APP_DIR:-$HOME/Applications}" ./build.sh "$VERSION"
APP="${TRAY_APP_DIR:-$HOME/Applications}/walgit-tray.app"

# 2. app 签名 + 公证 + 装订(通用脚本;已签已公证则幂等重做)
~/scripts/notarize.sh "$APP" --profile "$PROFILE"

# 3. 组装 DMG 暂存目录:app + /Applications 软链(拖放安装)
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

DMG="$DIST/walgit-${VERSION}-arm64.dmg"
rm -f "$DMG"
hdiutil create -volname "walgit" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null

# 4. DMG 签名 + 公证 + 装订
IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Developer ID Application/ {print $2; exit}')"
[ -n "$IDENTITY" ] || { echo "❌ Keychain 里没有 Developer ID Application 身份" >&2; exit 1; }
codesign --force --sign "$IDENTITY" --timestamp "$DMG"
xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait
xcrun stapler staple "$DMG"
spctl --assess --type open --context context:primary-signature -v "$DMG" 2>&1 | tail -2

echo ""
echo "✅ $DMG(签名 + 公证 + 装订)"
echo "上传:gh release upload <tag> $DMG --repo gqf2008/walgit"
