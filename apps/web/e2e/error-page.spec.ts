import { expect, test } from '@playwright/test'
import { registerUser, loginBrowser } from './helpers'
import { hydrated } from './support/hydrate'

// F-13：app/error.vue — 主題化繁中錯誤頁，SSR 狀態碼仍正確
test('未知路徑：HTTP 404 + 繁中錯誤頁 + 回首頁；console 無 NUXT_E1005', async ({ page, request }) => {
  const res = await request.get('/no-such-page-xyz')
  expect(res.status()).toBe(404)
  const html = await res.text()
  expect(html).toContain('<h1>找不到頁面</h1>') // 不是 Nuxt 預設英文頁
  const logs: string[] = []
  page.on('console', m => logs.push(m.text()))
  await page.goto('/no-such-page-xyz')
  await expect(page.getByRole('heading', { name: '找不到頁面' })).toBeVisible()
  await hydrated(page)
  await page.getByRole('button', { name: '回首頁' }).click()
  await expect(page).toHaveURL(/\/$/)
  expect(logs.filter(l => /NUXT_E1005/.test(l))).toEqual([])
})

test('非營運人員進 /admin：看起來像 404（依賴 middleware/staff 拋 403/404）', async ({ page, context }) => {
  const u = await registerUser()
  await loginBrowser(context, u)
  await page.goto('/admin')
  await expect(page.getByRole('heading', { name: '找不到頁面' })).toBeVisible()
  await u.api.dispose()
})
