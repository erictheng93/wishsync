# WishSync 願望清單（MVP）

建立願望清單、用連結分享，親友免註冊即可認領品項，並確保不會重複認領（超賣防護）。

## 結構

| 路徑 | 內容 |
|---|---|
| `apps/web` | Nuxt 4 前端（Cloudflare Pages；`/s/[slug]` 為 SSR 分享頁，供 LINE / FB 預覽） |
| `apps/api` | Rust（axum + sqlx）後端 |
| `db/migrations` | PostgreSQL 16 + PostGIS 遷移（MVP 14 張表） |
| `docs/` | PRD、使用者流程、線框、資料 schema 與 API 契約（v0.4） |
| `scripts/cargo.sh` | 沒安裝 Rust 時，用 Docker 執行 cargo |

## 本機開發

### 安裝 Rust（建議，編譯比 Docker 快很多）

```sh
brew install rustup && rustup-init -y     # 或：curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"
cargo --version                            # 確認安裝成功
```

可選的加速設定：在 `apps/api/Cargo.toml` 加上

```toml
[profile.dev]
debug = "line-tables-only"                 # 縮短連結時間
```

可選工具（`cargo binstall` 只用來下載現成工具的執行檔，不會加速專案本身的編譯）：

```sh
cargo install cargo-binstall
cargo binstall sqlx-cli cargo-watch        # sqlx-cli：管理 migration；cargo-watch：存檔自動重啟
```

沒安裝 Rust 時，仍可用 `scripts/cargo.sh test`（Docker）執行，只是較慢。

```sh
docker compose up -d                 # PostGIS、MinIO（模擬 R2）、Mailpit（收 OTP 信：http://localhost:8025）

# 後端（啟動時自動套用 migration）
export DATABASE_URL=postgres://wishsync:wishsync@localhost:5432/wishsync
export APP_ENV=dev   # 必填：未設即視為 production（缺機密 / 密鑰 <32 字元會拒絕啟動，見 apps/api/.env.example）
cargo run --manifest-path apps/api/Cargo.toml     # http://localhost:8080
# 沒有 Rust：scripts/cargo.sh test

# 前端
cd apps/web && npm install && npm run dev         # http://localhost:3000
```

## 測試

```sh
cargo test --manifest-path apps/api/Cargo.toml    # 每個測試使用獨立資料庫（sqlx::test）
cd apps/web && npm run typecheck && npm test && npm run build   # CI 的 web job 依序執行這三步
```

## 環境變數

- 後端：`DATABASE_URL`；其餘（SMTP、LINE Login 等）見 `apps/api/.env.example`
  - `BIND`：預設 `127.0.0.1:8080`（dev、production 皆是）。容器內部署需明確設 `BIND=0.0.0.0:8080`，且只讓 Cloudflare Tunnel / 內網可達。
  - `TRUSTED_PROXY`：`cloudflare` 或 `none`。dev 預設 `none`；**production 必須明確設定**，未設或其他值會拒絕啟動（不設會讓所有使用者看起來來自同一 IP，使每 IP 限流變成全域限流）。設 `cloudflare` 時信任 `CF-Connecting-IP`，所以 API 不可被直接存取，否則該標頭可被偽造；若 `BIND` 不是回送位址，啟動時會印出 WARN。
  - `TURNSTILE_SECRET`：production 必填；檢舉的 `turnstile_token` 會送 siteverify 驗證，失敗回 403 `FORBIDDEN`。dev/test 不設則略過驗證（接受任何 token，含前端的 `dev-bypass`）。`TURNSTILE_VERIFY_URL` 僅測試時覆寫。
  - `S3_ENDPOINT`、`S3_BUCKET`、`S3_ACCESS_KEY`、`S3_SECRET_KEY`、`S3_PUBLIC_BASE`：production 全部必填，缺漏啟動即 panic 並指出缺哪個；dev 預設本機 MinIO。`S3_REGION` 選填。
- 前端：`NUXT_PUBLIC_API_BASE`（預設 `http://localhost:8080`）；`NUXT_PUBLIC_TURNSTILE_SITE_KEY`（檢舉用 Cloudflare Turnstile；未設定時檢舉送 `dev-bypass`，僅限本機 mock；**正式部署必填**，`npm run deploy` 缺少它會直接中止，`deploy:demo` 不受影響）

## 部署

前端部署到 Cloudflare Pages，有兩種，差別在「建置時」的環境變數（`wrangler pages deploy` 不支援 `--config`，所以不用第二份 wrangler 設定）：

| 指令（在 `apps/web`） | 建置時設定 | 行為 |
|---|---|---|
| `npm run deploy`（正式） | `NUXT_DEMO` 清空；API 預設 `https://api.wishsync.tw` | 分享連結走真實 API，`/s/demo` 不會有假資料 |
| `npm run deploy:demo`（示範） | `NUXT_DEMO=1`、`NUXT_PUBLIC_API_BASE=` | 僅 `/s/demo` 有同源假資料，其餘分享連結失敗；**不要給真實使用者** |

- 正式 API 網域不同時，建置前設 `NUXT_PUBLIC_API_BASE=https://...`，或改 `nuxt.config.ts` 預設值。
- `wrangler.jsonc` 刻意不放 `vars`，避免執行期變數蓋掉建置設定。
- 本機驗證：`npm run build` 後 `npx wrangler pages dev dist`（示範用 `npm run build:demo`）。

## 開發順序

0. 骨架與 CI → 1. 技術原型 S1–S3（真實 Pages 帳號、LINE 預覽 / 內建瀏覽器 cookie / SSE，見 `docs/03` 第 14 節）
→ 2. 垂直切片：公開分享頁 → 訪客認領 → 創建者登入與編輯 → 進度儀表板 / 驚喜模式 / 即時更新 → MVP 附加功能
→ 3. 封閉測試 8 週，依 PRD 的 go / no-go 標準決定是否進入第二期。
