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
  routeRules: {
    ...Object.fromEntries(['/dashboard/**', '/lists/**', '/me/wallet', '/settings', '/friends', '/invite/**', '/u/**', '/admin/**', '/login', '/register', '/forgot-password'].map(r => [r, { ssr: false }])),
    // 測試用：NUXT_API_PROXY=<後端網址> 時，同網域的 /api/v1/** 代理到該後端（只有 npm run deploy:preview 會設）。
    // 為什麼：Pages 與臨時通道是不同網站，cookie 會被當第三方擋掉；同源代理後 cookie 是第一方。
    ...(process.env.NUXT_API_PROXY ? { '/api/v1/**': { proxy: `${process.env.NUXT_API_PROXY.replace(/\/$/, '')}/api/v1/**` } } : {}),
  },
  // PWA：手寫 manifest + public/sw.js（不用 @vite-pwa/nuxt，避免模組預設快取策略碰到即時資料與 SSR 分享頁）
  app: {
    head: {
      htmlAttrs: { lang: 'zh-Hant-TW' },
      meta: [
        { name: 'viewport', content: 'width=device-width, initial-scale=1, viewport-fit=cover' },
        { name: 'theme-color', content: '#c8102e', media: '(prefers-color-scheme: light)' },
        { name: 'theme-color', content: '#0e0e0e', media: '(prefers-color-scheme: dark)' },
        { name: 'apple-mobile-web-app-capable', content: 'yes' },
        { name: 'mobile-web-app-capable', content: 'yes' },
        { name: 'apple-mobile-web-app-status-bar-style', content: 'default' },
        { name: 'apple-mobile-web-app-title', content: 'WishSync' },
      ],
      link: [
        { rel: 'manifest', href: '/manifest.webmanifest' },
        { rel: 'apple-touch-icon', href: '/apple-touch-icon.png' },
      ],
    },
  },
})
