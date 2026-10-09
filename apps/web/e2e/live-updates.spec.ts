import { expect, test } from '@playwright/test'
import { claimViaUi, createList, loginBrowser, registerUser } from './helpers'

test('即時更新：創建者開著進度頁，訪客認領後不重新整理就看到變化', async ({ browser }) => {
  const owner = await registerUser()
  const list = await createList(owner, { showNames: true, items: [{ title: '推車', qty: 3 }, { title: '尿布', qty: 1 }] })
  const ownerCtx = await browser.newContext(), guestCtx = await browser.newContext()
  await loginBrowser(ownerCtx, owner)
  const op = await ownerCtx.newPage(), gp = await guestCtx.newPage()

  await test.step('創建者開進度頁（SSE 已連上、尚無認領）', async () => {
    await op.goto(`/lists/${list.id}/progress`)
    await expect(op.getByText('還沒有人認領')).toBeVisible()
    await expect(op.getByText('即時更新中')).toBeVisible()
    await op.evaluate(() => { (window as any).__noReload = true }) // 之後確認沒有整頁重載
  })

  await test.step('訪客在另一個 context 認領 2 件', async () => {
    await gp.goto(`/s/${list.slug}`)
    await claimViaUi(gp, '推車', '阿花', 2)
  })

  await test.step('創建者頁面自動出現認領者與更新後的進度', async () => {
    const card = op.getByRole('article').filter({ hasText: '推車' })
    await expect(card).toContainText('阿花 ×2')
    await expect(card).toContainText('2 / 3')
    await expect(op.getByText('數量 2 / 4')).toBeVisible()
    expect(await op.evaluate(() => (window as any).__noReload)).toBe(true)
  })

  await test.step('訪客取消後，創建者頁面再次自動更新', async () => {
    gp.once('dialog', d => d.accept())
    await gp.goto('/me/claims')
    await gp.getByRole('button', { name: '取消', exact: true }).click()
    await expect(op.getByText('數量 0 / 4')).toBeVisible()
  })
  await Promise.all([ownerCtx.close(), guestCtx.close(), owner.api.dispose()])
})

test('驚喜模式：進度頁遮蔽認領者，訪客頁正常', async ({ browser, request }) => {
  const owner = await registerUser()
  const list = await createList(owner, { surprise: true, items: [{ title: '生日蛋糕', qty: 2 }] })
  const ownerCtx = await browser.newContext(), guestCtx = await browser.newContext()
  await loginBrowser(ownerCtx, owner)
  const op = await ownerCtx.newPage(), gp = await guestCtx.newPage()

  await test.step('訪客頁正常：看得到進度、可以認領', async () => {
    await gp.goto(`/s/${list.slug}`)
    await expect(gp.locator('li.g-card', { hasText: '生日蛋糕' })).toContainText('0 / 2')
    await claimViaUi(gp, '生日蛋糕', '神秘朋友', 1)
    await gp.getByRole('button', { name: '回到清單繼續看' }).click()
    await expect(gp.locator('li.g-card', { hasText: '生日蛋糕' })).toContainText('1 / 2')
  })

  await test.step('創建者進度頁：只有整體完成度，看不到認領者與各品項進度', async () => {
    await op.goto(`/lists/${list.id}/progress`)
    await expect(op.getByRole('status').filter({ hasText: '驚喜模式' })).toBeVisible()
    await expect(op.getByText('進度已隱藏')).toBeVisible()
    await expect(op.getByText('神秘朋友')).toHaveCount(0)
    await expect(op.getByText(/數量 \d+ \/ \d+/)).toHaveCount(0)
  })

  await test.step('API 回應本身就不含認領者（不是只有前端遮罩）', async () => {
    const r = await owner.api.get(`wishlists/${list.id}/dashboard`)
    const body = await r.text()
    expect(body).not.toContain('神秘朋友')
    expect(JSON.parse(body).surprise_locked).toBe(true)
    const pub = await request.get(`${process.env.E2E_API ?? 'http://localhost:8081'}/api/v1/public/wishlists/${list.slug}`)
    expect(await pub.text()).not.toContain('神秘朋友')
  })

  await test.step('創建者開自己的分享頁：軟性遮蔽且不能認領', async () => {
    await op.goto(`/s/${list.slug}`)
    await expect(op.getByText('你正以建立者身分瀏覽')).toBeVisible()
    await expect(op.getByText('建立者不可認領自己的清單')).toBeVisible()
    await expect(op.getByRole('button', { name: '我要送' })).toHaveCount(0)
  })
  await Promise.all([ownerCtx.close(), guestCtx.close(), owner.api.dispose()])
})
