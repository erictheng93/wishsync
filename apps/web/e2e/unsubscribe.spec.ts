import { expect, test, type Route } from '@playwright/test'
import { hydrated } from './support/hydrate'

// F-26：退訂二步 — 開頁不送出，按下確認才 POST /unsubscribe
const cors = (route: Route) => ({ 'access-control-allow-origin': route.request().headers().origin ?? '*', 'access-control-allow-credentials': 'true' })
const POST = '**/api/v1/unsubscribe'

test('開啟連結不自動送出；按「確定退訂」後才 POST，成功顯示已退訂', async ({ page }) => {
  const posts: unknown[] = []
  await page.route(POST, async (route) => {
    if (route.request().method() === 'OPTIONS') return route.fallback()
    posts.push(route.request().postDataJSON())
    await route.fulfill({ status: 204, headers: cors(route) })
  })
  await page.goto('/unsubscribe?token=abc.def')
  await expect(page.getByRole('heading', { name: '確定要退訂認領通知？' })).toBeVisible()
  await page.waitForTimeout(800)
  expect(posts).toHaveLength(0)
  await hydrated(page)
  await page.getByRole('button', { name: '確定退訂' }).click()
  await expect(page.getByRole('heading', { name: '已退訂' })).toBeVisible()
  expect(posts).toEqual([{ token: 'abc.def' }])
})

test('無效 / 過期 token（422 INVALID_TOKEN）顯示說明', async ({ page }) => {
  await page.route(POST, route => route.request().method() === 'OPTIONS' ? route.fallback()
    : route.fulfill({ status: 422, contentType: 'application/json', body: JSON.stringify({ code: 'INVALID_TOKEN' }), headers: cors(route) }))
  await page.goto('/unsubscribe?token=bad')
  await hydrated(page)
  await page.getByRole('button', { name: '確定退訂' }).click()
  await expect(page.getByRole('heading', { name: '連結已失效' })).toBeVisible()
})

test('沒有 token 直接顯示失效；真實 API 對亂填 token 回 422 → 失效', async ({ page }) => {
  await page.goto('/unsubscribe')
  await expect(page.getByRole('heading', { name: '連結已失效' })).toBeVisible()
  await page.goto('/unsubscribe?token=not-a-real-token')
  await hydrated(page)
  await page.getByRole('button', { name: '確定退訂' }).click()
  await expect(page.getByRole('heading', { name: '連結已失效' })).toBeVisible()
})
