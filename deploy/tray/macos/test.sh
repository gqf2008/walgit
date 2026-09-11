#!/bin/bash
# macOS tray Release logic and package guard tests. Runs without starting the tray.
set -euo pipefail
cd "$(dirname "$0")"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/walgit-tray-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

cp release_logic_test_main.swift "$TMP/main.swift"
swiftc -swift-version 5 ReleaseLogic.swift "$TMP/main.swift" -o "$TMP/release-logic-tests"
"$TMP/release-logic-tests"

cat >"$TMP/walgit-good" <<'EOF'
#!/bin/sh
[ "${1:-}" = "--version" ] && echo "walgit v0.5.0"
EOF
cat >"$TMP/walgit-bad" <<'EOF'
#!/bin/sh
[ "${1:-}" = "--version" ] && echo "walgit v0.4.0"
EOF
chmod +x "$TMP/walgit-good" "$TMP/walgit-bad"
./build-dmg.sh --check-version "$TMP/walgit-good" 0.5.0
if ./build-dmg.sh --check-version "$TMP/walgit-bad" 0.5.0 >/dev/null 2>&1; then
    echo "FAIL: wrong version was accepted" >&2
    exit 1
fi

mkdir -p "$TMP/tree"
touch "$TMP/tree/._bad"
if ./build-dmg.sh --check-tree "$TMP/tree" >/dev/null 2>&1; then
    echo "FAIL: AppleDouble tree was accepted" >&2
    exit 1
fi
rm "$TMP/tree/._bad"
./build-dmg.sh --check-tree "$TMP/tree"

mkdir -p "$TMP/zip-src"
touch "$TMP/zip-src/._bad"
(cd "$TMP/zip-src" && zip -q "$TMP/bad.zip" ._bad)
if ./build-dmg.sh --check-zip "$TMP/bad.zip" >/dev/null 2>&1; then
    echo "FAIL: AppleDouble zip was accepted" >&2
    exit 1
fi
rm -f "$TMP/zip-src/._bad"
touch "$TMP/zip-src/good"
(cd "$TMP/zip-src" && zip -q "$TMP/good.zip" good)
./build-dmg.sh --check-zip "$TMP/good.zip"


pkginfo() { # pkginfo <app> <version>
    mkdir -p "$1/Contents/Resources"
    cat >"$1/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>com.walgit.tray.test</string>
    <key>CFBundleShortVersionString</key><string>$2</string>
    <key>CFBundleVersion</key><string>$2</string>
</dict>
</plist>
PLIST
    printf 'app-%s\n' "$2" >"$1/Contents/Resources/bundle-version"
}

stub_walgit() { # stub_walgit <deploy-dir> <version>
    cat >"$1/walgit" <<STUB
#!/bin/sh
[ "\${1:-}" = "--version" ] && echo "walgit v$2"
STUB
    chmod +x "$1/walgit"
}

# release_install_fixture <name> <old-version> <new-version> <success|rollback>
# 用一个必然不存在的 TRAY_PID 绕过真实托盘；WALGIT_UPDATE_SKIP_* 关闭
# 服务与 open 副作用。rollback 分支刻意不写新 marker/二进制，触发超时回滚。
release_install_fixture() {
    local name="$1"
    local old="$2"
    local new="$3"
    local expect="$4"
    local base="$TMP/fixture-$name"
    local deploy="$base/deploy"
    local dest="$base/Applications/walgit-tray.app"
    local mount="$base/mount"
    local dmg="$base/walgit-$new-arm64.dmg"
    rm -rf "$base"
    mkdir -p "$deploy" "$dest" "$mount/walgit-tray.app"
    pkginfo "$dest" "$old"
    stub_walgit "$deploy" "$old"
    printf '%s\n' "$old" >"$deploy/.skeleton-version"
    cp walgit-ensure "$deploy/walgit-ensure"
    chmod +x "$deploy/walgit-ensure"
    printf '#!/bin/sh\nexit 0\n' >"$deploy/run-walgit.sh"
    chmod +x "$deploy/run-walgit.sh"
    : >"$dmg"
    pkginfo "$mount/walgit-tray.app" "$new"
    if [ "$expect" = success ]; then
        # 模拟新 app bootstrap 完成：marker 与部署二进制都已是新版本。
        printf '%s\n' "$new" >"$deploy/.skeleton-version"
        stub_walgit "$deploy" "$new"
    fi

    local rc=0
    WALGIT_DEPLOY_DIR="$deploy" \
    WALGIT_UPDATE_SKIP_SERVICE=1 \
    WALGIT_UPDATE_SKIP_OPEN=1 \
    WALGIT_UPDATE_BOOTSTRAP_WAIT=3 \
    WALGIT_UPDATE_TRAY_WAIT=3 \
        ./release-install.sh "$dmg" "$mount" "$dest" "$new" 999999 >/dev/null 2>&1 || rc=$?

    if [ "$expect" = success ]; then
        [ "$rc" = 0 ] || { echo "FAIL($name): expected success, rc=$rc" >&2; return 1; }
        grep -qx "app-$new" "$dest/Contents/Resources/bundle-version" \
            || { echo "FAIL($name): app not replaced" >&2; return 1; }
        ls "$dest".bak-* >/dev/null 2>&1 \
            || { echo "FAIL($name): no app backup kept" >&2; return 1; }
        grep -qx "$new" "$deploy/.skeleton-version" \
            || { echo "FAIL($name): deploy marker not on new version" >&2; return 1; }
    else
        [ "$rc" != 0 ] || { echo "FAIL($name): expected rollback failure" >&2; return 1; }
        grep -qx "app-$old" "$dest/Contents/Resources/bundle-version" \
            || { echo "FAIL($name): old app not restored" >&2; return 1; }
        grep -qx "$old" "$deploy/.skeleton-version" \
            || { echo "FAIL($name): deploy marker not restored" >&2; return 1; }
        "$deploy/walgit" --version | grep -q "v$old" \
            || { echo "FAIL($name): deploy binary not restored" >&2; return 1; }
    fi
}


release_install_fixture success 0.4.0 0.5.0 success
release_install_fixture rollback 0.4.0 0.6.0 rollback

bash -n release-install.sh
echo "tray macos tests: ok"
