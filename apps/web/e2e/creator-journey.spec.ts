import { expect, test } from '@playwright/test'
import { newEmail, PASSWORD, skipOtpCooldown, uid, waitForOtp } from './helpers'

test('創建者旅程：註冊 → 建清單 → 新增品項 → 發佈 → 分享連結 → 登出/登入 → 忘記密碼', async ({ page, browser }) => {
  const email = newEmail('creator'), name = `阿美${uid().slice(0, 3)}`, title = `寶寶滿月 ${uid().slice(0, 4)}`
  const newPassword = 'brand-New-pass-9'
  let shareUrl = ''

  await test.step('註冊：填資料 → Mailpit 取 OTP → 驗證', async () => {
    await page.goto('/register')
    await page.getByLabel('顯示名稱').fill(name)
    await page.getByLabel('Email').fill(email)
    await page.getByLabel('密碼', { exact: true }).fill('short')
    await page.getByRole('button', { name: '寄送驗證碼' }).click()
    await expect(page.getByRole('alert')).toHaveText('密碼至少需 8 個字元') // 前端驗證
    await page.getByLabel('密碼', { exact: true }).fill(PASSWORD)
    await page.getByRole('button', { name: '寄送驗證碼' }).click()
    await expect(page.getByText(`驗證碼已寄到 ${email}`)).toBeVisible()
    // 先送錯誤的驗證碼
    await page.getByLabel('第 1 碼').fill('000000')
    await expect(page.getByRole('alert')).toContainText('驗證碼不正確或已過期')
    await page.getByLabel('第 1 碼').fill(await waitForOtp(email, /註冊/))
    await expect(page).toHaveURL(/\/dashboard$/)
    await expect(page.getByText('還沒有清單，建立第一份吧')).toBeVisible()
  })

  await test.step('建立清單', async () => {
    await page.getByRole('link', { name: '＋ 建立清單' }).click()
    await page.getByLabel(/清單名稱/).fill(title)
    await page.getByRole('button', { name: '建立並新增品項' }).click()
    await expect(page).toHaveURL(/\/lists\/[^/]+\/edit$/)
    await expect(page.getByText('還沒有品項，先新增第一個吧')).toBeVisible()
    await expect(page.getByRole('button', { name: '發佈並分享' })).toBeDisabled() // 沒有品項不能發佈
  })

  await test.step('新增品項', async () => {
    await page.getByRole('button', { name: '＋ 新增品項' }).click()
    const sheet = page.getByRole('dialog')
    await sheet.getByLabel('名稱（必填）').fill('奶瓶')
    await sheet.getByLabel(/需要數量/).fill('2')
    await sheet.getByRole('button', { name: '儲存品項' }).click()
    await expect(page.getByRole('article').filter({ hasText: '奶瓶' })).toContainText('需要 2')
  })

  await test.step('發佈並取得分享連結', async () => {
    await page.getByRole('button', { name: '發佈並分享' }).click()
    await expect(page.getByRole('dialog', { name: '分享你的清單' })).toBeVisible()
    shareUrl = await page.locator('#c-share-input').inputValue()
    expect(shareUrl).toMatch(/\/s\/[A-Za-z0-9]{10}$/)
  })

  await test.step('分享連結可以被另一位（未登入）訪客開啟', async () => {
    const ctx = await browser.newContext()
    const guest = await ctx.newPage()
    await guest.goto(shareUrl)
    await expect(guest.getByRole('heading', { level: 1, name: title })).toBeVisible()
    await expect(guest.locator('li.g-card', { hasText: '奶瓶' })).toContainText('0 / 2')
    await ctx.close()
  })

  await test.step('登出', async () => {
    await page.goto('/settings')
    await page.getByRole('button', { name: '登出' }).click()
    await expect(page).toHaveURL(/\/login$/)
    await page.goto('/dashboard')
    await expect(page).toHaveURL(/\/login\?redirect=(%2F|\/)dashboard$/) // 未登入被導回
  })

  await test.step('錯誤密碼：顯示錯誤訊息且不洩漏帳號是否存在', async () => {
    await page.getByLabel('Email').fill(email)
    await page.getByLabel('密碼').fill('totally-wrong-pw')
    await page.getByRole('button', { name: '登入', exact: true }).click()
    await expect(page.getByRole('alert')).toHaveText('帳號或密碼錯誤')
    await page.getByLabel('Email').fill(newEmail('nobody'))
    await page.getByRole('button', { name: '登入', exact: true }).click()
    await expect(page.getByRole('alert')).toHaveText('帳號或密碼錯誤') // 不存在的帳號回相同訊息
  })

  await test.step('密碼登入成功，並導回原本要去的頁面', async () => {
    await page.getByLabel('Email').fill(email)
    await page.getByLabel('密碼').fill(PASSWORD)
    await page.getByRole('button', { name: '登入', exact: true }).click()
    await expect(page).toHaveURL(/\/dashboard$/)
    await expect(page.getByRole('link', { name: title })).toBeVisible()
  })

  await test.step('忘記密碼：OTP → 設新密碼 → 用新密碼登入，舊密碼失效', async () => {
    await page.goto('/settings')
    await page.getByRole('button', { name: '登出' }).click()
    skipOtpCooldown(email) // 註冊剛寄過 OTP；同 email 60 秒內不會再寄（見 README）
    await page.goto('/forgot-password')
    await page.getByLabel('Email').fill(email)
    await page.getByRole('button', { name: '寄送驗證碼' }).click()
    await expect(page.getByText('若帳號存在，已寄出驗證碼')).toBeVisible()
    await page.getByLabel('第 1 碼').fill(await waitForOtp(email, /重設密碼/))
    await page.getByLabel('新密碼').fill(newPassword)
    await page.getByRole('button', { name: '重設密碼' }).click()
    await expect(page).toHaveURL(/\/login\?reset=1$/)
    await expect(page.getByText('密碼已重設，請用新密碼登入')).toBeVisible()

    await page.getByLabel('Email').fill(email)
    await page.getByLabel('密碼').fill(PASSWORD)
    await page.getByRole('button', { name: '登入', exact: true }).click()
    await expect(page.getByRole('alert')).toHaveText('帳號或密碼錯誤')
    await page.getByLabel('密碼').fill(newPassword)
    await page.getByRole('button', { name: '登入', exact: true }).click()
    await expect(page).toHaveURL(/\/dashboard$/)
  })
})
