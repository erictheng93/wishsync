import { expect, test, type Page, type Route } from '@playwright/test'
import { createList, registerUser } from './helpers'
import { hydrated } from './support/hydrate'

// F-05 / F-01 防呆 / F-10：訪客認領的錯誤處理（瀏覽器端請求用 page.route 注入錯誤；SSR 仍打真實 API）
const CLAIMS = '**/api/v1/items/*/claims'
const TOKEN_KEY = 'ws_guest_token'
const ls = (page: Page) => page.evaluate(k => localStorage.getItem(k), TOKEN_KEY)

let slug: string
test.beforeAll(async () => {
  const owner = await registerUser()
  slug = (await createList(owner, { items: [{ title: '奶瓶組', qty: 5 }] })).slug
  await owner.api.dispose()
})
async function open(page: Page, staleToken = false) {
  const me = page.waitForResponse(/\/guest\/me/) // 頁面載入時的 /guest/me 若 401 本來就會清掉 token，所以等它回來再放入失效 token
  await page.goto(`/s/${slug}`)
  await hydrated(page)
  await me
  if (staleToken) await page.evaluate(([k]) => localStorage.setItem(k, 'stale-token-value'), [TOKEN_KEY])
  await page.getByRole('button', { name: '我要送' }).click()
  await page.getByRole('dialog').getByLabel('你的暱稱（必填）').fill('阿明')
}
const submit = (page: Page) => page.getByRole('button', { name: '確認認領' }).click()
const json = (route: Route, status: number, body: object) => route.fulfill({
  status, contentType: 'application/json', body: JSON.stringify(body),
  headers: { 'access-control-allow-origin': route.request().headers().origin ?? '*', 'access-control-allow-credentials': 'true' },
})

test('失效的 guest token：401 → 清掉 token、以新訪客自動重試一次並成功', async ({ page }) => {
  const seen: (string | undefined)[] = []
  await page.route(CLAIMS, async (route) => {
    if (route.request().method() === 'OPTIONS') return route.fallback()
    seen.push(route.request().headers()['x-guest-token'])
    if (seen.length === 1) return json(route, 401, { code: 'UNAUTHORIZED' }) // 沒有 detail
    return route.fallback()
  })
  await open(page, true)
  await submit(page)
  await expect(page.getByRole('heading', { name: '✓ 認領成功！' })).toBeVisible()
  expect(seen).toHaveLength(2)
  expect(seen[0]).toBe('stale-token-value')
  expect(seen[1]).toBeUndefined()
  const t = await ls(page)
  expect(t).toBeTruthy(); expect(t).not.toBe('stale-token-value')
})

test('重試後仍 401：顯示有資訊的訊息（不是「發生錯誤，請稍後再試」），token 已清除', async ({ page }) => {
  await page.route(CLAIMS, route => route.request().method() === 'OPTIONS' ? route.fallback() : json(route, 401, { code: 'UNAUTHORIZED' }))
  await open(page, true)
  await submit(page)
  const alert = page.getByRole('dialog').getByRole('alert')
  await expect(alert).toContainText('身分已失效')
  await expect(alert).not.toContainText('發生錯誤，請稍後再試')
  expect(await ls(page)).toBeNull()
})

for (const [status, body, text] of [
  [404, { code: 'NOT_FOUND' }, '這個品項已不存在'],
  [410, { code: 'WISHLIST_REMOVED' }, '這份清單已被下架'],
  [429, { code: 'RATE_LIMITED' }, '稍後再試'],
  [503, { code: 'SERVICE_UNAVAILABLE' }, '系統維護中'],
] as const) {
  test(`${status} 沒有 detail 時依 code / 狀態顯示「${text}」`, async ({ page }) => {
    await page.route(CLAIMS, route => route.request().method() === 'OPTIONS' ? route.fallback() : json(route, status, body))
    await open(page)
    await submit(page)
    await expect(page.getByRole('dialog').getByRole('alert')).toContainText(text)
  })
}

test('201 但沒有 guest_token 且本機沒有 token：成功頁顯示警告，不謊稱已存在此裝置', async ({ page }) => {
  await page.route(CLAIMS, async (route) => {
    if (route.request().method() === 'OPTIONS') return route.fallback()
    const res = await route.fetch()
    const body = await res.json()
    delete body.guest_token
    await route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify(body), headers: { 'access-control-allow-origin': route.request().headers().origin ?? '*', 'access-control-allow-credentials': 'true' } })
  })
  await open(page)
  await submit(page)
  const ok = page.getByRole('dialog', { name: '✓ 認領成功！' })
  await expect(ok).toContainText('沒有保存到身分')
  await expect(ok).not.toContainText('已存在此裝置')
})

test('暱稱 maxlength 為 30；欄位錯誤只出現一次（橫幅不重複）', async ({ page }) => {
  await page.route(CLAIMS, route => route.request().method() === 'OPTIONS' ? route.fallback()
    : json(route, 422, { code: 'VALIDATION_FAILED', detail: '暱稱需為 1–30 字', errors: [{ pointer: '/display_name', detail: '暱稱需為 1–30 字' }] }))
  await open(page)
  const nick = page.getByRole('dialog').getByLabel('你的暱稱（必填）')
  await expect(nick).toHaveAttribute('maxlength', '30')
  await submit(page)
  await expect(page.getByText('暱稱需為 1–30 字')).toHaveCount(1)
})
