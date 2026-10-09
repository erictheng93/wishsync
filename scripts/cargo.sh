#!/usr/bin/env sh
# 本機沒裝 Rust 時用 Docker 跑 cargo：scripts/cargo.sh test
# 連到 docker compose 的 db（host.docker.internal:5432）
cd "$(dirname "$0")/../apps/api" || exit 1
exec docker run --rm -v "$PWD/../..":/w -w /w/apps/api \
  -v wishsync-cargo:/usr/local/cargo/registry -v wishsync-target:/w/apps/api/target \
  -e DATABASE_URL="${DATABASE_URL:-postgres://wishsync:wishsync@host.docker.internal:5432/wishsync}" \
  -e SQLX_OFFLINE=false rust:1 cargo "$@"
