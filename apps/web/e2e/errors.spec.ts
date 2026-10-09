import { expect, test } from '@playwright/test'
import { spawn, type ChildProcess } from 'node:child_process'
import { createList, registerUser } from './helpers'
import { API } from './support/env'

test('不存在的 slug：404 狀態碼與 404 頁面', async ({ page, request }) => {
  for (const slug of ['AbCdEfGhIj', 'x']) { // 長度正確但不存在 / 長度不符
    const res = await request.get(`/s/${slug}`)
    expect(res.status(), slug).toBe(404)
    expect(await res.text()).toContain('找不到這份清單')
  }
  await page.goto('/s/AbCdEfGhIj')
  await expect(page.getByRole('heading', { name: '找不到這份清單' })).toBeVisible()
  await page.getByRole('link', { name: '回首頁' }).click()
  await expect(page).toHaveURL(/\/$/)
})

test('草稿（未發佈）清單對訪客等同不存在', async ({ request }) => {
  const owner = await registerUser()
  const list = await createList(owner, { publish: false })
  expect((await request.get(`/s/${list.slug}`)).status()).toBe(404)
  await owner.api.dispose()
})

test.describe('後端不可用', () => {
  let web: ChildProcess
  const PORT = 3013
  test.beforeAll(async () => {
    // SSR 請求是 Nuxt 伺服器對 API 發的，page.route 攔不到；所以另起一個「API 指向死連接埠」的 Web 實例（共用同一份 build）
    web = spawn('node', ['e2e/.app/.output/server/index.mjs'], {
      env: { ...process.env, PORT: String(PORT), HOST: '127.0.0.1', NUXT_PUBLIC_API_BASE: 'http://127.0.0.1:1' }, stdio: 'ignore',
    })
    await expect.poll(async () => fetch(`http://127.0.0.1:${PORT}/login`).then(r => r.status, () => 0), { timeout: 20_000 }).toBe(200)
  })
  test.afterAll(() => { web?.kill() })

  test('SSR：API 連不上 → HTTP 503 + 「系統暫時無法載入」+ noindex（不會被快取成「不存在」）', async ({ page, request }) => {
    const url = `http://127.0.0.1:${PORT}/s/AbCdEfGhIj`
    const res = await request.get(url)
    expect(res.status()).toBe(503)
    const html = await res.text()
    expect(html).toContain('系統暫時無法載入，請稍後再試')
    expect(html).toContain('noindex')
    await page.goto(url)
    await expect(page.getByRole('heading', { name: '系統暫時無法載入，請稍後再試' })).toBeVisible()
    await expect(page.getByRole('button', { name: '重新載入' })).toBeVisible()
  })
})

test('瀏覽器端：API 回 500 時頁面不白屏（已載入的內容保留）', async ({ page }) => {
  // 限制：page.route 只能攔瀏覽器發出的請求，SSR 那一次無法攔截——所以這裡驗證的是「載入後」的 client refresh 行為
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '保留的品項', qty: 1 }] })
  await page.goto(`/s/${list.slug}`)
  await expect(page.locator('li.g-card', { hasText: '保留的品項' })).toBeVisible()
  await page.route(`${API}/api/v1/public/wishlists/${list.slug}`, r => r.fulfill({ status: 500, contentType: 'application/problem+json', body: '{"code":"INTERNAL_ERROR"}' }))
  await page.evaluate(() => window.dispatchEvent(new Event('online'))) // 觸發 refresh()
  await page.waitForTimeout(1000)
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await expect(page.locator('li.g-card', { hasText: '保留的品項' })).toBeVisible()
  await owner.api.dispose()
})
