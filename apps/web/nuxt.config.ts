// 部署：Cloudflare Pages（docs D48，使用者基於成本選擇）
export default defineNuxtConfig({
  compatibilityDate: '2026-10-01',
  nitro: { preset: 'cloudflare_pages' },
  runtimeConfig: {
    // 建置時讀取：NUXT_DEMO=1 才是 S1 示範（/s/demo 回固定資料），只有 npm run deploy:demo 會設
    demo: process.env.NUXT_DEMO ?? '',
    // 建置（production）預設指向正式 API；dev 預設本機。可用 NUXT_PUBLIC_API_BASE 覆寫（建置時或 Pages 後台變數）
    public: { apiBase: process.env.NUXT_PUBLIC_API_BASE ?? (process.env.NODE_ENV === 'production' ? 'https://api.wishsync.tw' : 'http://localhost:8080'), turnstileSiteKey: '' },
  },
  // 創建者頁面靠 cookie 驗證，只在 client 渲染；/s/** 維持 SSR（OG 預覽）
  routeRules: Object.fromEntries(['/dashboard/**', '/lists/**', '/settings', '/admin/**', '/login', '/register', '/forgot-password'].map(r => [r, { ssr: false }])),
  app: { head: { htmlAttrs: { lang: 'zh-Hant-TW' } } },
})
