// 正式部署前檢查：缺少 Turnstile 網站金鑰時，檢舉功能會全部失敗（前端送 dev-bypass，後端回 403），所以直接擋下部署。
// Nuxt 也會讀 .env，所以這裡一併載入（不覆蓋已存在的環境變數）。
try { process.loadEnvFile('.env') } catch { /* 沒有 .env 也可以 */ }

const missing = ['NUXT_PUBLIC_TURNSTILE_SITE_KEY'].filter(k => !process.env[k]?.trim())
if (missing.length) {
  console.error(`\n✘ 正式部署缺少環境變數：${missing.join(', ')}\n  建置時沒有它，檢舉功能上線後無法使用。請在 Cloudflare 建立 Turnstile 小工具取得網站金鑰後再部署。\n`)
  process.exit(1)
}
