import { expect, test } from '@playwright/test'

test('登入頁有三種方式；Google / LINE 未設憑證 → oauth_unavailable', async ({ page }) => {
  await page.goto('/login')
  await expect(page.getByRole('button', { name: '登入', exact: true })).toBeVisible() // Email + 密碼
  await expect(page.getByLabel('Email')).toBeVisible()
  await expect(page.getByRole('button', { name: '使用 Google 繼續' })).toBeVisible()
  await expect(page.getByRole('button', { name: '使用 LINE 繼續' })).toBeVisible()

  for (const [btn, provider] of [['使用 Google 繼續', 'google'], ['使用 LINE 繼續', 'line']]) {
    await test.step(`${provider}：沒有憑證時導回登入頁並說明`, async () => {
      await page.goto('/login')
      await page.getByRole('button', { name: btn }).click()
      await expect(page).toHaveURL(/\/login\?error=oauth_unavailable/)
      await expect(page.getByRole('alert')).toHaveText('此登入方式尚未開放')
    })
  }

  await test.step('oauth_failed 顯示失敗訊息', async () => {
    await page.goto('/login?error=oauth_failed')
    await expect(page.getByRole('alert')).toHaveText('第三方登入失敗，請重試')
  })
})
