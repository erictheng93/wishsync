// 部署：Cloudflare Pages（docs D48，使用者基於成本選擇）
export default defineNuxtConfig({
  compatibilityDate: '2026-10-01',
  nitro: { preset: 'cloudflare_pages' },
  runtimeConfig: {
    demo: '', // NUXT_DEMO=1：S1 預覽部署，/s/demo 回固定資料
    // 覆寫：NUXT_PUBLIC_API_BASE
    public: { apiBase: 'http://localhost:8080', turnstileSiteKey: '' },
  },
  // 創建者頁面靠 cookie 驗證，只在 client 渲染；/s/** 維持 SSR（OG 預覽）
  routeRules: Object.fromEntries(['/dashboard/**', '/lists/**', '/settings', '/admin/**', '/login', '/register', '/forgot-password'].map(r => [r, { ssr: false }])),
  app: { head: { htmlAttrs: { lang: 'zh-Hant-TW' } } },
})
