import { defineConfig, devices } from '@playwright/test'
import { API, WEB } from './e2e/support/env'

const CI = !!process.env.CI
// iPhone 13 裝置參數（390 寬、觸控、行動 UA），但只裝了 chromium，所以拿掉 webkit 的 defaultBrowserType
const { defaultBrowserType: _ignored, ...iphone } = devices['iPhone 13']

// 啟動順序：webServer 先於 globalSetup，所以「重置 e2e 資料庫」必須在 API 啟動前完成（API 啟動時才會套 migration）。
// 因此重置寫在 API 的啟動指令裡（reset-db.mjs），而不是 globalSetup。
// Web 用 production build + node-server preview（不用 nuxt dev：沒有 HMR / 依賴最佳化重載造成的 flaky，
// 也不會跟開發中的 .nuxt 互相覆蓋——e2e/.app 是獨立 buildDir 的 layer）。
export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.spec.ts',
  outputDir: 'test-results',
  // 每個測試自建唯一資料（唯一 email / 清單），彼此不共享狀態，所以可平行。
  // 限流都以「身分」計（email / guest / user），或每 IP 但額度遠大於整個測試集的用量（見 README）；
  // 所以不需要 workers=1。
  fullyParallel: true,
  workers: CI ? 2 : 3,
  retries: CI ? 2 : 0,
  forbidOnly: CI,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: WEB,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'off',
    locale: 'zh-TW',
    timezoneId: 'Asia/Taipei',
  },
  projects: [
    { name: 'desktop-chromium', use: { ...devices['Desktop Chrome'] } },
    // 行動版只跑與版面 / 觸控最相關的案例，其餘流程與桌面相同，避免重複消耗時間
    { name: 'mobile', testMatch: /(guest-journey|smoke|line-browser)\.spec\.ts/, use: { ...iphone, browserName: 'chromium' } },
  ],
  webServer: [
    {
      name: 'api',
      command: 'cargo build --manifest-path ../api/Cargo.toml && node e2e/support/reset-db.mjs && ../api/target/debug/wishsync-api',
      url: `${API}/api/v1/me`, // 未登入回 401；Playwright 視 2xx/3xx/400-403 為已啟動
      ignoreHTTPSErrors: true,
      reuseExistingServer: false,
      timeout: 600_000,
      stdout: 'pipe', stderr: 'pipe',
      env: {
        APP_ENV: 'dev',
        DATABASE_URL: process.env.E2E_DATABASE_URL ?? 'postgres://wishsync:wishsync@localhost:5432/wishsync_e2e',
        BIND: new URL(API).host,
        APP_URL: WEB,
        RUST_LOG: 'warn',
      },
    },
    {
      name: 'web',
      command: 'npm run build:e2e && PORT=3012 HOST=127.0.0.1 node e2e/.app/.output/server/index.mjs',
      url: `${WEB}/login`,
      reuseExistingServer: false,
      timeout: 300_000,
      stdout: 'pipe', stderr: 'pipe',
      env: { NUXT_PUBLIC_API_BASE: API },
    },
  ],
})
