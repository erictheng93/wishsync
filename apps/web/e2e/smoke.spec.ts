import { expect, test, type Page } from '@playwright/test'
import { createList, guestClaim, loginBrowser, makeStaff, registerUser, watchProblems } from './helpers'

// 主要頁面在 390 寬：無橫向捲動、無 console error / pageerror、無 4xx/5xx 請求（預期內的除外）
test.use({ viewport: { width: 390, height: 844 } })

const noHScroll = (page: Page) => page.evaluate(() => ({ sw: document.documentElement.scrollWidth, iw: window.innerWidth }))

async function check(page: Page, path: string, ready: () => Promise<void>, ignore: RegExp[] = []) {
  const problems = watchProblems(page, ignore)
  await page.goto(path)
  await ready()
  await page.waitForLoadState('load')
  const { sw, iw } = await noHScroll(page)
  expect.soft(sw, `${path} 橫向捲動 (scrollWidth ${sw} > ${iw})`).toBeLessThanOrEqual(iw)
  expect.soft(problems, `${path} 的 console error / 失敗請求`).toEqual([])
}

test('公開頁面（訪客可見）', async ({ page }) => {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '一個名字很長很長很長很長很長很長很長很長很長很長的品項名稱用來測試換行', qty: 5 }, { title: 'B', qty: 1 }] })
  await guestClaim(list.items[1].id, 1)
  const h1 = () => expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  for (const [path, ready] of [
    ['/', h1], ['/login', h1], ['/register', h1], ['/forgot-password', h1], ['/terms', h1], ['/privacy', h1],
    [`/s/${list.slug}`, h1], ['/me/claims', h1],
  ] as const) {
    await test.step(path, () => check(page, path, ready))
  }
  await test.step('/s/{slug} 的認領 sheet 與成功畫面', async () => {
    await page.goto(`/s/${list.slug}`)
    await page.locator('li.g-card').first().getByRole('button', { name: '我要送' }).click()
    const { sw, iw } = await noHScroll(page)
    expect.soft(sw).toBeLessThanOrEqual(iw)
    await expect(page.getByRole('dialog')).toBeVisible()
    const box = await page.getByRole('dialog').boundingBox()
    expect(box!.x).toBeGreaterThanOrEqual(0)
    expect(box!.x + box!.width).toBeLessThanOrEqual(iw)
  })
  await owner.api.dispose()
})

test('創建者與後台頁面', async ({ page, context }) => {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '奶瓶', qty: 2 }] })
  await guestClaim(list.items[0].id, 1, '訪客甲')
  makeStaff(owner.email)
  await loginBrowser(context, owner)
  const h1 = () => expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  for (const path of ['/dashboard', '/lists/new', `/lists/${list.id}/edit`, `/lists/${list.id}/progress`, '/settings', '/admin']) {
    await test.step(path, () => check(page, path, h1))
  }
})
