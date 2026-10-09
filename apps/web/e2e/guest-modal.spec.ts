import { expect, test, type Page } from '@playwright/test'
import { createList, registerUser } from './helpers'
import { hydrated } from './support/hydrate'

// F-02：認領 sheet / 成功 overlay / 檢舉 sheet 是真正的 modal（原生 <dialog>）
// 原生 modal：Tab 到尾會離開頁面進瀏覽器 UI（activeElement 變 body），但不會跑到被遮住的背景頁
const inDialog = (page: Page) => page.evaluate(() => { const a = document.activeElement; return a === document.body || !!a?.closest('dialog[open]') })
const focusedInDialog = (page: Page) => page.evaluate(() => !!document.activeElement?.closest('dialog[open]'))

test.describe('modal 鍵盤行為', () => {
  let slug: string
  test.beforeAll(async () => {
    const owner = await registerUser()
    slug = (await createList(owner, { items: [{ title: '奶瓶組', qty: 2 }] })).slug
    await owner.api.dispose()
  })

  test('認領 sheet：初始焦點在暱稱、Tab 不離開、Esc 關閉、焦點回到觸發按鈕、有 accessible name', async ({ page }) => {
    await page.goto(`/s/${slug}`)
    await hydrated(page)
    const trigger = page.getByRole('button', { name: '我要送' })
    await trigger.focus()
    await trigger.click()
    const dialog = page.getByRole('dialog', { name: /認領「奶瓶組」/ })
    await expect(dialog).toBeVisible()
    await expect(dialog.getByLabel('你的暱稱（必填）')).toBeFocused()
    for (let i = 0; i < 16; i++) { await page.keyboard.press('Tab'); expect(await inDialog(page), `Tab #${i + 1}`).toBe(true) }
    for (let i = 0; i < 16; i++) { await page.keyboard.press('Shift+Tab'); expect(await inDialog(page), `Shift+Tab #${i + 1}`).toBe(true) }
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog')).toHaveCount(0)
    await expect(trigger).toBeFocused()
  })

  test('認領 sheet：已輸入內容時 Esc / 點背景要二次確認，取消確認則不關閉', async ({ page }) => {
    await page.goto(`/s/${slug}`)
    await hydrated(page)
    await page.getByRole('button', { name: '我要送' }).click()
    const dialog = page.getByRole('dialog')
    await dialog.getByLabel('你的暱稱（必填）').fill('小明')
    const asked: string[] = []
    page.once('dialog', d => { asked.push(d.message()); void d.dismiss() })
    await page.keyboard.press('Escape')
    await expect.poll(() => asked.length).toBe(1)
    await expect(dialog).toBeVisible()
    expect(await dialog.getByLabel('你的暱稱（必填）').inputValue()).toBe('小明')
    page.once('dialog', d => { asked.push(d.message()); void d.dismiss() })
    await page.mouse.click(4, 4) // 背景
    await expect.poll(() => asked.length).toBe(2)
    await expect(dialog).toBeVisible()
    page.once('dialog', d => void d.accept())
    await page.mouse.click(4, 4)
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })

  test('沒輸入內容時點背景直接關閉', async ({ page }) => {
    await page.goto(`/s/${slug}`)
    await hydrated(page)
    await page.getByRole('button', { name: '我要送' }).click()
    await expect(page.getByRole('dialog')).toBeVisible()
    await page.mouse.click(4, 4)
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })

  test('檢舉 sheet：Esc 關閉並還焦點；成功 overlay：焦點在 dialog 內、Esc 關閉', async ({ page }) => {
    await page.goto(`/s/${slug}`)
    await hydrated(page)
    const report = page.getByRole('button', { name: '檢舉此清單' })
    await report.click()
    const rd = page.getByRole('dialog', { name: '檢舉此清單' })
    await expect(rd).toBeVisible()
    expect(await focusedInDialog(page)).toBe(true)
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog')).toHaveCount(0)
    await expect(report).toBeFocused()

    await page.getByRole('button', { name: '我要送' }).click()
    await page.getByRole('dialog').getByLabel('你的暱稱（必填）').fill('阿華')
    await page.getByRole('button', { name: '確認認領' }).click()
    const ok = page.getByRole('dialog', { name: '✓ 認領成功！' })
    await expect(ok).toBeVisible()
    expect(await focusedInDialog(page)).toBe(true)
    for (let i = 0; i < 8; i++) { await page.keyboard.press('Tab'); expect(await inDialog(page), `overlay Tab #${i + 1}`).toBe(true) }
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })
})
