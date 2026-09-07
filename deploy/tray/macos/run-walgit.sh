#!/usr/bin/env bash
# walgit 启动脚本:凭证可选——.r2-credentials / .walgit_token 存在才读
# (全新部署走 /setup 向导配置 store,不需要凭证文件)。
set -euo pipefail
cd "$(dirname "$0")"
if [ -f "$(dirname "$0")/.r2-credentials" ]; then
    source "$(dirname "$0")/.r2-credentials"
    export R2_ACCESS_KEY R2_SECRET_KEY
fi
if [ -f "$(dirname "$0")/.walgit_token" ]; then
    export WALGIT_TOKEN=$(cat "$(dirname "$0")/.walgit_token")
fi
exec "$(dirname "$0")/walgit" serve --config "$(dirname "$0")/walgit.toml" "$@"
