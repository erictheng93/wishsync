import { expect, test } from '@playwright/test'
import { createList, registerUser } from './helpers'
import { hydrated } from './support/hydrate'

// F-04 長字串不溢位；F-19 無 JS 時說明 + 隱藏「我要送」
const W = (n: number) => 'W'.repeat(n)
let slug: string
test.beforeAll(async () => {
  const owner = await registerUser()
  const l = await createList(owner, { title: W(100), items: [{ title: W(120), qty: 2 }] })
  const cur = await (await owner.api.get(`wishlists/${l.id}`)).json()
  await owner.api.patch(`wishlists/${l.id}`, { data: { description: W(300), expected_updated_at: cur.wishlist.updated_at } })
  await owner.api.patch(`items/${l.items[0].id}`, { data: { brand: W(60), spec: W(60) } })
  slug = l.slug
  await owner.api.dispose()
})

for (const width of [320, 390, 1440]) {
  test(`分享頁長字串在 ${width}px 不水平溢位`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 })
    await page.goto(`/s/${slug}`)
    await hydrated(page)
    await expect(page.getByRole('heading', { level: 1 })).toContainText(W(20))
    const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(over).toBeLessThanOrEqual(0)
  })
}

test.describe('無 JavaScript', () => {
  test.use({ javaScriptEnabled: false })
  test('內容可看、顯示說明、「我要送」不顯示', async ({ page }) => {
    await page.goto(`/s/${slug}`)
    await expect(page.getByText('需要 JavaScript 才能認領；你仍可查看清單內容')).toBeVisible()
    await expect(page.getByRole('button', { name: '我要送' })).toBeHidden()
    await expect(page.getByRole('heading', { level: 1 })).toContainText(W(20))
  })
})

test('有 JS 時不顯示無 JS 說明', async ({ page }) => {
  await page.goto(`/s/${slug}`)
  await hydrated(page)
  await expect(page.getByRole('button', { name: '我要送' })).toBeVisible()
  await expect(page.getByText('需要 JavaScript 才能認領')).toBeHidden()
})
