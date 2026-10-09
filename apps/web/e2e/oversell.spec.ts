import { expect, test } from '@playwright/test'
import { createList, registerUser } from './helpers'

test('防超賣：兩位訪客同時搶最後 1 件，恰好 1 位成功', async ({ browser, request }) => {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '限量玩偶', qty: 1 }] })

  // 兩個獨立 context = 兩位互不相識的訪客（各自的 localStorage / cookie）
  const [ca, cb] = await Promise.all([browser.newContext(), browser.newContext()])
  const [a, b] = await Promise.all([ca.newPage(), cb.newPage()])
  const pages = [a, b]
  const success = (p: typeof a) => p.getByRole('heading', { name: '✓ 認領成功！' })
  const lost = (p: typeof a) => p.getByRole('alert').getByText('剛剛被別人認領完了')

  await test.step('兩人都打開分享頁並填好認領表單（尚未送出）', async () => {
    await Promise.all(pages.map(async (p, i) => {
      await p.goto(`/s/${list.slug}`)
      await p.locator('li.g-card', { hasText: '限量玩偶' }).getByRole('button', { name: '我要送' }).click()
      await p.getByLabel('你的暱稱（必填）').fill(`搶購者${i}`)
    }))
  })

  await test.step('同時按下確認認領', async () => {
    await Promise.all(pages.map(p => p.getByRole('button', { name: '確認認領' }).click()))
  })

  await test.step('恰好一人成功、另一人看到「剛剛被別人認領完了」', async () => {
    await expect.poll(async () => (await success(a).isVisible()) + (await success(b).isVisible())).toBe(1)
    const [winner, loser] = (await success(a).isVisible()) ? [a, b] : [b, a]
    await expect(lost(loser)).toBeVisible()
    await expect(success(loser)).toHaveCount(0)
    await expect(loser.getByRole('dialog')).toBeHidden() // 約 1.2 秒後自動關閉
    await expect(loser.locator('li.g-card', { hasText: '限量玩偶' })).toContainText('已被認領完')
    await expect(loser.locator('li.g-card', { hasText: '限量玩偶' }).getByRole('button', { name: '我要送' })).toHaveCount(0)
    await winner.getByRole('button', { name: '回到清單繼續看' }).click()
    await expect(winner.locator('li.g-card', { hasText: '限量玩偶' })).toContainText('1 / 1')
  })

  await test.step('伺服器端數量沒有超賣（qty_claimed = qty_needed = 1）', async () => {
    const r = await request.get(`${process.env.E2E_API ?? 'http://localhost:8081'}/api/v1/public/wishlists/${list.slug}`)
    const item = (await r.json()).items[0]
    expect(item.qty_claimed).toBe(1)
    expect(item.is_fully_claimed).toBe(true)
  })
  await Promise.all([ca.close(), cb.close(), owner.api.dispose()])
})

test('防超賣：頁面開著時別人搶走最後 1 件，頁面自動更新為「已被認領完」（SSE / 輪詢）', async ({ page }) => {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '最後一個', qty: 1 }] })
  await page.goto(`/s/${list.slug}`)
  const card = page.locator('li.g-card', { hasText: '最後一個' })
  await expect(page.getByText('即時更新中')).toBeVisible() // SSE 已連上
  await expect(card.getByRole('button', { name: '我要送' })).toBeVisible()
  const { guestClaim } = await import('./helpers')
  await guestClaim(list.items[0].id, 1)
  await expect(card).toContainText('已被認領完')
  await owner.api.dispose()
})
