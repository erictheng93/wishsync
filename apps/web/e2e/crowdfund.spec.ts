import { expect, test, type Browser, type Page } from '@playwright/test'
import { createList, loginBrowser, makeStaff, registerUser, sql, type User } from './helpers'

// P2-A 點數眾籌：真實 API + Web + DB，跑完「設定 → 發點 → 贊助 → 撤回 → 達標 → 捐款者名單 → 採購 → 差額退回」與「截止後轉投」。
// 每個測試自建使用者與清單，可平行。

const ADDR = { recipient_name: '王小米', phone: '0912345678', address: '台北市大安區和平東路二段 1 號 3 樓' }
const inDays = (d: number) => new Date(Date.now() + d * 864e5).toISOString()
/** datetime-local 的值（台北時間） */
const taipeiInput = (d: number) => new Date(Date.now() + d * 864e5 + 8 * 36e5).toISOString().slice(0, 16)

async function pageAs(browser: Browser, u: User): Promise<Page> {
  const ctx = await browser.newContext()
  await loginBrowser(ctx, u)
  return ctx.newPage()
}
async function grant(staff: User, user: User, points: number) {
  const me = await (await user.api.get('me')).json()
  const r = await staff.api.post('admin/wallets/grants', { data: { user_id: me.id, points, reason: 'E2E 發點' }, headers: { 'Idempotency-Key': crypto.randomUUID() } })
  expect(r.status(), await r.text()).toBe(201)
}
async function pledge(u: User, itemId: string, points: number) {
  const r = await u.api.post(`items/${itemId}/contributions`, { data: { points }, headers: { 'Idempotency-Key': crypto.randomUUID() } })
  expect(r.status(), await r.text()).toBe(201)
  return (await r.json()).contribution.id as string
}
async function cfItem(owner: User, listId: string, title: string, target: number) {
  const r = await owner.api.post(`wishlists/${listId}/items`, { data: { title, funding_mode: 'crowdfund', target_points: target, funding_deadline: inDays(7), priority: 'medium' } })
  expect(r.status(), await r.text()).toBe(201)
  return (await r.json()).id as string
}
const card = (page: Page, title: string) => page.locator('li.g-card', { hasText: title })
const balanceOf = (page: Page) => page.locator('.c-big')

test('眾籌完整旅程：收件資訊 → 建立眾籌品項 → 發點 → 贊助 / 撤回 → 達標 → 捐款者名單 → 採購與差額退回', async ({ browser }) => {
  test.slow()
  const [owner, staff, amy, ben] = await Promise.all([
    registerUser({ name: '小米媽媽', tag: 'cfo' }), registerUser({ name: '營運小王', tag: 'cfs' }),
    registerUser({ name: '阿美', tag: 'cfa' }), registerUser({ name: '阿本', tag: 'cfb' }),
  ])
  makeStaff(staff.email)
  const list = await createList(owner, { title: `推車眾籌 ${Date.now()}`, showNames: true, items: [{ title: '奶瓶', qty: 2 }], publish: false })

  // ---- 建立者：沒填收件資訊時不能建眾籌品項；填完後建立並發佈 ----
  const op = await pageAs(browser, owner)
  await op.goto(`/lists/${list.id}/edit`)
  const fillCrowdfund = async () => {
    await op.getByRole('button', { name: '＋ 新增品項' }).click()
    await op.getByLabel('名稱（必填）').fill('嬰兒推車')
    await op.getByLabel('點數眾籌').check()
    await op.getByLabel(/目標點數/).fill('1000')
    await op.getByLabel(/募集截止/).fill(taipeiInput(7))
    await op.getByRole('button', { name: '儲存品項' }).click()
  }
  await fillCrowdfund()
  await expect(op.getByRole('alert')).toContainText('收件資訊')
  await op.getByRole('button', { name: '填寫收件資訊' }).click()
  await op.getByLabel(/收件人/).fill(ADDR.recipient_name)
  await op.getByLabel(/電話/).fill(ADDR.phone)
  await op.getByLabel(/地址/).fill(ADDR.address)
  await op.getByRole('button', { name: '儲存收件資訊' }).click()
  await expect(op.getByText('已儲存')).toBeVisible()
  await expect(op.getByText('王*米')).toBeVisible() // 只顯示遮罩
  await fillCrowdfund()
  await expect(op.getByRole('button', { name: '儲存品項' })).toBeHidden()
  await op.getByRole('button', { name: '發佈並分享' }).click()
  await expect.poll(async () => (await (await owner.api.get(`wishlists/${list.id}`)).json()).wishlist.status).toBe('active')
  const slug = (await (await owner.api.get(`wishlists/${list.id}`)).json()).wishlist.slug as string
  const stroller = (await (await owner.api.get(`wishlists/${list.id}`)).json()).items.find((i: any) => i.title === '嬰兒推車').id as string
  // DB 只有密文
  expect(sql(`SELECT position(convert_to('王小米', 'UTF8') in recipient_name_enc) + position(convert_to('和平東路', 'UTF8') in address_enc) FROM shipping_addresses WHERE wishlist_id = '${list.id}'`)).toBe('0')

  // ---- 營運：在後台幫阿美發點；阿本用 API 發 ----
  const sp = await pageAs(browser, staff)
  await sp.goto('/admin/wallets')
  await sp.getByLabel('搜尋使用者').fill(amy.email)
  await sp.getByRole('button', { name: '搜尋' }).click()
  await sp.locator('article', { hasText: amy.email }).getByRole('button', { name: '發點 / 扣點' }).click()
  await sp.getByLabel(/點數（正數發放/).fill('2000')
  await sp.getByLabel(/原因（必填/).fill('E2E 轉帳入點')
  await sp.getByRole('button', { name: '下一步' }).click()
  await sp.getByRole('button', { name: '確認發放' }).click()
  await expect(sp.locator('article', { hasText: amy.email })).toContainText('2,000')
  await grant(staff, ben, 1000)

  // ---- 未登入訪客按贊助 → 導去登入 ----
  const guest = await (await browser.newContext()).newPage()
  await guest.goto(`/s/${slug}`)
  await card(guest, '嬰兒推車').getByRole('button', { name: '用點數贊助' }).click()
  await expect(guest).toHaveURL(/\/login\?redirect=/)

  // ---- 阿美：在分享頁贊助 400，再到「我的點數」撤回 ----
  const ap = await pageAs(browser, amy)
  await ap.goto(`/s/${slug}`)
  await expect(card(ap, '嬰兒推車')).toContainText('已募 0 / 1,000 點')
  await card(ap, '嬰兒推車').getByRole('button', { name: '用點數贊助' }).click()
  const fund = ap.getByRole('dialog')
  await fund.getByLabel('贊助點數').fill('400')
  await fund.getByLabel('留言（選填）').fill('寶寶加油')
  await fund.getByRole('button', { name: /確認贊助/ }).click()
  await expect(ap.getByRole('heading', { name: '✓ 感謝你的贊助！' })).toBeVisible()
  await expect(ap.getByText('錢包餘額剩 1,600 點')).toBeVisible()
  await ap.getByRole('link', { name: '查看我的點數與認捐' }).click()
  await expect(balanceOf(ap)).toHaveText('1,600')
  await expect(ap.getByText('認捐中 400 點')).toBeVisible()
  await ap.getByRole('button', { name: '撤回' }).click()
  await ap.getByRole('button', { name: '確認撤回' }).click()
  await expect(balanceOf(ap)).toHaveText('2,000')
  await expect(ap.locator('article', { hasText: '嬰兒推車' })).toContainText('已退回')
  await pledge(amy, stroller, 600)

  // ---- 阿本：匿名贊助剩餘的 400，剛好達標 ----
  const bp = await pageAs(browser, ben)
  await bp.goto(`/s/${slug}`)
  await card(bp, '嬰兒推車').getByRole('button', { name: '用點數贊助' }).click()
  const bf = bp.getByRole('dialog')
  await bf.getByLabel('贊助點數').fill('400')
  await bf.getByLabel('匿名贊助（頁面上顯示為「匿名朋友」）').check()
  await bf.getByRole('button', { name: /確認贊助/ }).click()
  await expect(bp.getByRole('heading', { name: '✓ 感謝你的贊助！' })).toBeVisible()
  await expect(bp.getByText('剛好達標了！')).toBeVisible()
  await bp.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(bp, '嬰兒推車')).toContainText('已達標')

  // ---- 建立者進度頁：捐款者名單（誰、多少、何時、達標時間）----
  await op.goto(`/lists/${list.id}/progress`)
  const row = op.locator('article', { hasText: '嬰兒推車' })
  await expect(row.locator('li', { hasText: '阿美' })).toContainText('600 點')
  await expect(row.locator('li', { hasText: '阿美' })).toContainText('認捐時間')
  await expect(row.locator('li', { hasText: '阿美' })).toContainText('達標時間')
  await expect(row.locator('li', { hasText: '匿名朋友' })).toContainText('400 點')
  await expect(row).not.toContainText('阿本') // 匿名者不揭露
  await expect(row).toContainText('已達標扣用')
  await expect(row).toContainText('待下單')

  // ---- 營運：採購單 → 下單（實際 900，差額 100 依比例退回）→ 出貨 → 送達 ----
  await sp.goto('/admin/orders')
  await sp.getByLabel('狀態').selectOption({ label: '全部' }) // 預設只看待下單（工作佇列），推進後會離開該篩選
  const order = sp.locator('article', { hasText: '嬰兒推車' })
  await expect(order).toContainText(ADDR.address) // staff 看得到明文收件資訊
  await order.getByRole('button', { name: '標記已下單' }).click()
  await sp.getByLabel(/商家訂單編號/).fill('SHOP-E2E-1')
  await sp.getByLabel(/實際金額/).fill('900')
  await sp.getByRole('dialog').getByRole('button', { name: '標記已下單' }).click()
  await expect(sp.getByRole('status')).toContainText('狀態已更新為「已下單」，差額 100 點已退回捐贈者')
  await expect(order).toContainText('SHOP-E2E-1')
  await order.getByRole('button', { name: '標記已出貨' }).click()
  await sp.getByLabel(/物流單號/).fill('TW123456789')
  await sp.getByRole('dialog').getByRole('button', { name: '標記已出貨' }).click()
  await expect(order).toContainText('TW123456789')
  expect(Number(sql(`SELECT count(*) FROM audit_logs WHERE action = 'order.view_address'`))).toBeGreaterThan(0)

  // 差額退回：600 / 400 → 60 / 40
  await ap.goto('/me/wallet')
  await expect(balanceOf(ap)).toHaveText('1,460')
  await expect(ap.getByText('差額退回').first()).toBeVisible()
  await expect(ap.getByText('已退回差額 60 點')).toBeVisible()
  await bp.goto('/me/wallet')
  await expect(balanceOf(bp)).toHaveText('640')

  // 建立者看到採購進度
  await op.reload()
  await expect(row).toContainText('TW123456789')
  await expect(row).toContainText('出貨時間')
  await sp.goto('/admin/orders')
  await sp.getByLabel('狀態').selectOption({ label: '運送中' })
  await order.getByRole('button', { name: '標記已送達' }).click()
  await sp.getByRole('dialog').getByRole('button', { name: '標記已送達' }).click()
  await expect(sp.getByRole('status')).toContainText('狀態已更新為「已送達」')
  await bp.goto(`/s/${slug}`)
  await expect(card(bp, '嬰兒推車')).toContainText('已送達')

  // 對帳：每個錢包 balance = Σ ledger.delta
  expect(sql(`SELECT count(*) FROM point_wallets w WHERE balance <> (SELECT COALESCE(sum(delta), 0) FROM point_ledger l WHERE l.wallet_id = w.id)`)).toBe('0')
})

test('截止未達標：捐贈者在 7 天選擇期內轉投到同清單其他品項', async ({ browser }) => {
  const [owner, staff, amy] = await Promise.all([registerUser({ tag: 'rao' }), registerUser({ tag: 'ras' }), registerUser({ name: '阿美', tag: 'raa' })])
  makeStaff(staff.email)
  const list = await createList(owner, { publish: false })
  expect((await owner.api.put(`wishlists/${list.id}/shipping-address`, { data: ADDR })).status()).toBe(200)
  const a = await cfItem(owner, list.id, '汽車座椅', 5000)
  await cfItem(owner, list.id, '嬰兒床', 3000)
  const cur = await (await owner.api.get(`wishlists/${list.id}`)).json()
  expect((await owner.api.patch(`wishlists/${list.id}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } })).status()).toBe(200)
  await grant(staff, amy, 1000)
  await pledge(amy, a, 700)
  sql(`UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() WHERE id = '${a}'`) // 模擬截止 job 已執行

  const ap = await pageAs(browser, amy)
  await ap.goto('/me/wallet')
  const c = ap.locator('article', { hasText: '汽車座椅' })
  await expect(c).toContainText('已截止')
  await expect(c).toContainText('逾期會自動退回')
  await c.getByRole('button', { name: '轉投其他品項' }).click()
  await ap.getByLabel('轉投到').selectOption({ label: '嬰兒床（還差 3,000 點）' })
  await ap.getByRole('button', { name: '確認轉投' }).click()
  await expect(ap.locator('article', { hasText: '嬰兒床' })).toContainText('已認捐')
  await expect(ap.locator('article', { hasText: '汽車座椅' })).toContainText('已轉投')
  await expect(balanceOf(ap)).toHaveText('300') // 轉投不動錢包
})
