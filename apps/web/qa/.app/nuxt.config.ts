// QA 專用 Nuxt layer：獨立 buildDir / .output（不碰 e2e/.app 與 apps/web/.nuxt），node-server preset
export default defineNuxtConfig({
  extends: ['../..'],
  nitro: { preset: 'node-server' },
})
