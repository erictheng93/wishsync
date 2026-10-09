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

### 端對端測試（Playwright）

`apps/web/e2e/` 以真實的 API + Web + Postgres + Mailpit 跑瀏覽器流程（訪客認領、防超賣、創建者註冊 / 登入 / 發佈、即時更新、驚喜模式、檢舉下架、LINE 內建瀏覽器、版面煙霧測試等）。

前置條件：
- `docker compose up -d db mailpit`（Postgres 在 5432、Mailpit 在 8025；OTP 信從 Mailpit HTTP API 讀取）。
- 已安裝 Rust（`cargo` 在 PATH 上）與 Node 22。第一次執行：`cd apps/web && npm ci && npx playwright install chromium`。
- 本機不需要安裝 `psql`：會改用 `docker compose exec db psql`（CI 則用本機 psql）。
- 不要設定 `GOOGLE_CLIENT_ID` / `LINE_CHANNEL_ID` 等憑證環境變數（「未設憑證」的登入案例依賴它們不存在）。

```sh
cd apps/web
npm run e2e            # 全部（desktop-chromium + mobile 兩個 project）
npm run e2e:ui         # Playwright UI 模式
npm run e2e:headed     # 顯示瀏覽器
npx playwright test guest-journey --project=desktop-chromium   # 單一檔案
npx playwright show-report                                      # 開啟上次的 HTML 報告
```

隔離方式（不會干擾開發環境）：
- 資料庫 `wishsync_e2e`：每次執行前由 `e2e/support/reset-db.mjs` DROP 後重建，API 啟動時自動套用 migrations；開發用的 `wishsync` 資料庫不受影響。
- API 用 `127.0.0.1:8081`、Web 用 `127.0.0.1:3012`（`playwright.config.ts` 的 `webServer` 啟動，且不重用既有伺服器；連接埠被佔用會直接失敗）。Web 是 production build + node-server（`e2e/.app` 是獨立 buildDir 的 Nuxt layer，不會覆蓋開發中的 `.nuxt`）。
- 每個測試自建唯一資料（唯一 email、唯一清單），所以可平行（本機 3 workers、CI 2）。後端限流以身分計（email / guest / user），每 IP 的額度（新訪客 100/小時、檢舉 10/小時、登入失敗 30/15 分鐘）遠大於整個測試集的用量；後端不信任 `X-Forwarded-For`，所以測試也不靠它。
- 同一 email 的 OTP 有 60 秒間隔（登入 / 註冊 / 重設共用），需要連續寄第二封時，測試用 `skipOtpCooldown(email)` 把舊紀錄往前推。
- staff 權限由 `makeStaff(email)` 以 psql 直接設定 `users.is_staff`。

失敗時看 `apps/web/playwright-report/`（`trace: on-first-retry`，CI 會重試 2 次並把報告與 trace 上傳為 artifact）。

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


### 臨時公開測試（本機後端 + 快速通道 + Pages 預覽分支）

用來在手機或 LINE 上測試本機的後端。**這會把你電腦上的 API 經臨時網址公開**，請用獨立的資料庫與隨機機密，測完就關。

1. 後端：用獨立資料庫、隨機的 `OTP_PEPPER` / `OAUTH_SECRET` / `UNSUB_SECRET`，`TRUSTED_PROXY=cloudflare`、`BIND=127.0.0.1:8090`、`APP_URL=https://qa.wishsync-web.pages.dev`。
2. 通道：`cloudflared tunnel --url http://127.0.0.1:8090`，取得 `https://xxx.trycloudflare.com`。
3. 前端：`NUXT_API_PROXY=https://xxx.trycloudflare.com npm run deploy:preview`（部署到 `qa` 預覽分支，不動正式網址；前端以同源代理連後端，cookie 才是第一方）。
4. 限制：**快速通道不支援 SSE**，即時更新會自動退回每 15 秒輪詢；通道網址每次重開都會變，需要重新部署。要驗證 SSE 需使用有網域的具名通道。

## 開發順序

0. 骨架與 CI → 1. 技術原型 S1–S3（真實 Pages 帳號、LINE 預覽 / 內建瀏覽器 cookie / SSE，見 `docs/03` 第 14 節）
→ 2. 垂直切片：公開分享頁 → 訪客認領 → 創建者登入與編輯 → 進度儀表板 / 驚喜模式 / 即時更新 → MVP 附加功能
→ 3. 封閉測試 8 週，依 PRD 的 go / no-go 標準決定是否進入第二期。
