# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

WishSync：願望清單，用連結分享，訪客免註冊即可認領品項，防超賣。文件與程式註解以繁體中文為主。完整設定與部署細節見 `README.md`；產品/契約規格在 `docs/`（PRD、流程、線框、`04-data-schema-api-contract.html` 的 API 契約，程式註解常以「契約 x.x」「D48」引用）。

## 指令

```sh
docker compose up -d                       # PostGIS、MinIO（模擬 R2）、Mailpit（OTP 信 http://localhost:8025）

# 後端 apps/api（Rust, axum + sqlx）；啟動時自動套用 db/migrations
export DATABASE_URL=postgres://wishsync:wishsync@localhost:5432/wishsync
export APP_ENV=dev                         # 必設：未設視為 production，缺機密會拒絕啟動
cargo run --manifest-path apps/api/Cargo.toml          # :8080
cargo test --manifest-path apps/api/Cargo.toml         # sqlx::test，每個測試獨立資料庫（需 DB 在跑）
cargo test --manifest-path apps/api/Cargo.toml --test claims <name>   # 單一測試檔 / 單一測試
scripts/cargo.sh test                      # 沒裝 Rust 時用 Docker

# 前端 apps/web（Nuxt 4）
cd apps/web && npm install && npm run dev  # :3000
npm run typecheck && npm test && npm run build   # CI 依序執行；單一測試：npx vitest run tests/errors.test.ts
node mocks/server.mjs                      # 訪客端契約 mock，:8080（slug: demo/closed/gone/boom/sseoff），不需後端/DB
```

## 架構

**後端**（`apps/api/src`，lib `wishsync_api`）：每個模組各自提供 `routes()`，由 `lib.rs::app()` 合併到 `/api/v1`；新增模組只改那裡。全域 middleware：`read_only`（`system_flags.read_only` 為真時非安全方法回 503，auth/admin system-flags 例外）與 CORS（只允許 `APP_URL` 並帶憑證）。`config.rs` 是全域單例（`config::init/get`），production 缺機密、`TRUSTED_PROXY`、S3 設定會啟動即失敗。

- **兩種身分**：創建者用 cookie `ws_session`（`session::CurrentUser` extractor，DB 只存 SHA-256）；訪客用 `X-Guest-Token` header 優先、其次 `ws_guest` cookie（`guest.rs`，`Actor::Guest|User`）。
- **認領（`claims.rs`）**：核心是超賣防護。鎖序固定 `wishlist_items`（條件式 UPDATE / FOR UPDATE）→ `claims`，改動時不可顛倒。寫入需 `Idempotency-Key`（UUID），`idempotency.rs` 的占位與業務寫入在同一交易，失敗 rollback 不留痕；重放回應帶 `Idempotency-Replayed`。逾期認領會釋放，另有 `ratelimit.rs`（DB 計數，背景清理）。
- 背景工作在 `main.rs` 啟動：`notify::spawn_worker`（寄信）、`ratelimit::spawn_cleanup`。錯誤統一為 RFC 9457 problem+json（`error.rs`，含 `code` 與 `errors[]`）。
- 測試在 `apps/api/tests/*.rs`（依功能切片命名），皆用 `#[sqlx::test(migrations = "../../db/migrations")]`。
- DB：PostgreSQL 16 + PostGIS，`db/migrations/000N_*.sql` 依序累加。

**前端**（`apps/web`）：
- `/s/[slug]` 為 SSR 分享頁（LINE/FB 預覽）；創建者頁面（dashboard、lists、settings、admin、登入等）在 `nuxt.config.ts` 的 `routeRules` 設為 `ssr: false`（靠 cookie 驗證），新增創建者路由要加進該清單。
- API 在不同網域：`useApi()`（創建者，`credentials: 'include'`，把錯誤正規化為 `CreatorApiError`，401 自動導向登入）與 `useGuest()`（訪客權杖）。即時更新 `useWishlistEvents` 用 SSE，連續失敗 2 次改輪詢。
- 部署 Cloudflare Pages。`NUXT_DEMO` 與 `NUXT_PUBLIC_API_BASE` 是**建置時**變數：`npm run deploy`（正式）與 `deploy:demo`（只有 `/s/demo` 假資料，`server/routes/api/v1/public/...` 僅此時啟用，勿給真實使用者）；`wrangler.jsonc` 刻意不放 `vars`。
- 樣式為「紅白實驗室」主題（白底、紅 #c8102e、直角、警示斜紋進度條）；token 在 `assets/tokens.css`，參考 `design/themes.html` 款式二。
