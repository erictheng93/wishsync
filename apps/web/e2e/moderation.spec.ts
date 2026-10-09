import { expect, test } from '@playwright/test'
import { API } from './support/env'
import { createList, loginBrowser, makeStaff, registerUser } from './helpers'

test('檢舉與下架：訪客檢舉 → staff 後台下架 → 410 → 復原', async ({ browser, request }) => {
  const owner = await registerUser()
  const list = await createList(owner, { title: `可疑清單 ${Date.now()}` })
  const staff = await registerUser({ tag: 'staff' })
  makeStaff(staff.email)
  const guestCtx = await browser.newContext(), staffCtx = await browser.newContext(), ownerCtx = await browser.newContext()
  await loginBrowser(staffCtx, staff); await loginBrowser(ownerCtx, owner)
  const gp = await guestCtx.newPage(), sp = await staffCtx.newPage(), op = await ownerCtx.newPage()

  await test.step('訪客檢舉（dev 無 Turnstile site key → dev-bypass）', async () => {
    await gp.goto(`/s/${list.slug}`)
    await gp.getByRole('button', { name: '檢舉此清單' }).click()
    const dlg = gp.getByRole('dialog')
    await dlg.getByLabel('不當內容').check()
    await dlg.getByLabel('補充說明（選填）').fill('E2E 測試檢舉')
    await dlg.getByRole('button', { name: '送出檢舉' }).click()
    await expect(gp.getByText('已收到檢舉，我們會盡快處理')).toBeVisible()
  })

  await test.step('staff 後台的檢舉佇列看得到這份清單', async () => {
    await sp.goto('/admin')
    const report = sp.getByRole('article').filter({ hasText: list.slug })
    await expect(report).toContainText('不當內容')
    await expect(report).toContainText('E2E 測試檢舉')
    await expect(report).toContainText('檢舉者：anonymous')
    await report.getByRole('button', { name: '下架' }).click()
    const dlg = sp.getByRole('dialog')
    await expect(dlg.getByRole('button', { name: '下架此清單' })).toBeDisabled() // 原因必填
    await dlg.getByLabel(/下架原因/).fill('E2E：違反條款')
    await dlg.getByRole('button', { name: '下架此清單' }).click()
    await expect(sp.getByText('已下架', { exact: true })).toBeVisible()
  })

  await test.step('/s/{slug} 回 410，顯示已下架、noindex、不洩漏標題', async () => {
    const res = await request.get(`/s/${list.slug}`)
    expect(res.status()).toBe(410)
    const html = await res.text()
    expect(html).toContain('此清單已被下架')
    expect(html).toContain('noindex')
    expect(html).not.toContain(list.title)
    await gp.reload()
    await expect(gp.getByRole('heading', { name: '此清單已被下架' })).toBeVisible()
    expect((await request.get(`${API}/api/v1/public/wishlists/${list.slug}`)).status()).toBe(410)
    expect((await request.get(`${API}/api/v1/public/wishlists/${list.slug}/events`)).status()).toBe(410)
  })

  await test.step('創建者在儀表板看到下架說明與原因', async () => {
    await op.goto('/dashboard')
    await expect(op.getByRole('alert').filter({ hasText: '這份清單已被下架' })).toContainText('E2E：違反條款')
  })

  await test.step('staff 以清單搜尋恢復上架 → 公開頁恢復', async () => {
    await sp.getByRole('button', { name: '清單搜尋' }).click()
    await sp.getByLabel('搜尋清單').fill(list.slug)
    await sp.getByRole('button', { name: '搜尋', exact: true }).click()
    const row = sp.getByRole('article').filter({ hasText: list.slug })
    await row.getByRole('button', { name: '恢復上架' }).click()
    await sp.getByRole('dialog').getByRole('button', { name: '恢復上架' }).click()
    await expect(sp.getByText('已恢復上架')).toBeVisible()
    const res = await request.get(`/s/${list.slug}`)
    expect(res.status()).toBe(200)
    await gp.reload()
    await expect(gp.getByRole('heading', { level: 1, name: list.title })).toBeVisible()
  })

  await test.step('非 staff 進不了 /admin', async () => {
    await op.goto('/admin')
    await expect(op.getByText('營運後台')).toHaveCount(0)
  })
  await Promise.all([guestCtx.close(), staffCtx.close(), ownerCtx.close(), owner.api.dispose(), staff.api.dispose()])
})
