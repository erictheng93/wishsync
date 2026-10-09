// e2e 專用的 Nuxt layer：沿用 apps/web 的全部設定與程式碼，但 buildDir / 輸出在這個資料夾，
// 並用 node-server preset（正式設定是 cloudflare_pages），不會動到開發中的 .nuxt。
export default defineNuxtConfig({
  extends: ['../..'],
  nitro: { preset: 'node-server' },
})
