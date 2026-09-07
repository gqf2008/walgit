#!/usr/bin/env bash
# walgit 启动脚本：凭证从 ~/walgit/.r2-credentials + .walgit_token 读取（R2 后端）
set -euo pipefail
cd "$(dirname "$0")"
source "$(dirname "$0")/.r2-credentials"
export R2_ACCESS_KEY R2_SECRET_KEY
export WALGIT_TOKEN=$(cat "$(dirname "$0")/.walgit_token")
exec "$(dirname "$0")/walgit" serve --config "$(dirname "$0")/walgit.toml" "$@"
