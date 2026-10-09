import { expect, test } from '@playwright/test'
import { createList, registerUser, waitForMail } from './helpers'

const LINE_UA = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari Line/14.5.0'

test.describe('LINE 內建瀏覽器', () => {
  test.use({ userAgent: LINE_UA })

  test('顯示「用預設瀏覽器開啟」引導橫幅，關閉後記住', async ({ page }) => {
    const owner = await registerUser()
    const list = await createList(owner)
    await page.goto(`/s/${list.slug}`)
    const banner = page.locator('.g-banner', { hasText: '用預設瀏覽器開啟' })
    await expect(banner).toBeVisible()
    await banner.getByRole('button', { name: '知道了' }).click()
    await expect(banner).toBeHidden()
    await page.reload()
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
    await expect(banner).toBeHidden()
    await owner.api.dispose()
  })

  test('localStorage 被禁用：仍可認領，改用 cookie 備援保存', async ({ page }) => {
    const owner = await registerUser()
    const list = await createList(owner)
    await page.addInitScript(() => {
      Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('The operation is insecure.', 'SecurityError') } })
    })
    await page.goto(`/s/${list.slug}`)
    const card = page.locator('li.g-card', { hasText: '奶瓶' })
    await card.getByRole('button', { name: '我要送' }).click()
    await page.getByLabel('你的暱稱（必填）').fill('LINE 使用者')
    await page.getByRole('button', { name: '確認認領' }).click()
    await expect(page.getByRole('heading', { name: '✓ 認領成功！' })).toBeVisible()
    await expect(page.getByText('LINE 內建瀏覽器關閉後，認領紀錄可能找不到')).toBeVisible()
    await page.getByRole('button', { name: '回到清單繼續看' }).click()
    await page.reload() // cookie 備援讓重新整理後仍認得
    await expect(card.getByRole('button', { name: '修改我的認領' })).toBeVisible()
    await owner.api.dispose()
  })

  test('所有本機儲存都不可用：認領仍成功，提示截圖保存；留 Email 則寄管理連結', async ({ page }) => {
    const owner = await registerUser()
    const list = await createList(owner)
    await page.addInitScript(() => {
      const deny = () => { throw new DOMException('The operation is insecure.', 'SecurityError') }
      Object.defineProperty(window, 'localStorage', { get: deny })
      Object.defineProperty(window, 'sessionStorage', { get: deny })
      Object.defineProperty(Document.prototype, 'cookie', { get: () => '', set: () => {} })
    })
    await page.goto(`/s/${list.slug}`)
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible() // 沒有因為 storage 例外而壞掉
    await page.locator('li.g-card', { hasText: '奶瓶' }).getByRole('button', { name: '我要送' }).click()
    const dlg = page.getByRole('dialog')
    const email = `e2e-line-${Date.now()}@example.com`
    await dlg.getByLabel('你的暱稱（必填）').fill('無儲存的人')
    await dlg.getByLabel(/^Email（選填）/).fill(email)
    await dlg.getByRole('button', { name: '確認認領' }).click()
    await expect(page.getByRole('heading', { name: '✓ 認領成功！' })).toBeVisible()
    await expect(page.getByText('無法儲存於此裝置，請截圖保存')).toBeVisible()
    await expect(page.getByText(/已寄出管理連結到 e\*\*\*@example\.com/)).toBeVisible()
    const mail = await waitForMail(email, { subject: /認領已確認/ })
    expect(mail.text).toMatch(/\/me\/claims#r=[\w-]+/) // 管理連結
    await owner.api.dispose()
  })
})
