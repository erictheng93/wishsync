import { expect, test } from '@playwright/test'
import { createList, registerUser } from './helpers'

test('訪客旅程：SSR meta → 認領 → 重新整理 → 我的認領 → 修改 → 取消', async ({ page, request }) => {
  const owner = await registerUser({ name: '小愛' })
  const list = await createList(owner, { title: '小愛的待產清單', items: [{ title: '奶瓶組', qty: 3 }, { title: '嬰兒帽', qty: 1 }] })
  const card = page.locator('li.g-card', { hasText: '奶瓶組' })
  const dialog = page.getByRole('dialog')

  await test.step('SSR 原始 HTML 含 OG meta（爬蟲看得到，不依賴 JS）', async () => {
    const res = await request.get(`/s/${list.slug}`)
    expect(res.status()).toBe(200)
    const html = await res.text()
    const meta = (p: string) => html.match(new RegExp(`<meta[^>]+(?:property|name)="${p}"[^>]+content="([^"]*)"`))?.[1]
    expect(meta('og:title')).toBe('幫小愛挑禮物｜小愛的待產清單')
    expect(meta('og:description')).toContain('不用註冊')
    expect(meta('og:type')).toBe('website')
    expect(meta('og:url')).toContain(`/s/${list.slug}`)
    expect(meta('og:image')).toMatch(/^https?:\/\/.+\.(png|jpg)/)
    expect(meta('twitter:card')).toBe('summary_large_image')
    expect(html).toContain('<h1>小愛的待產清單</h1>') // 內容也在首次 HTML 內
  })

  await test.step('開啟分享頁，看到兩個品項與 0% 進度', async () => {
    await page.goto(`/s/${list.slug}`)
    await expect(page.getByRole('heading', { level: 1, name: '小愛的待產清單' })).toBeVisible()
    await expect(page.locator('li.g-card')).toHaveCount(2)
    await expect(card).toContainText('0 / 3')
  })

  await test.step('暱稱必填：空白被瀏覽器擋下、純空白被前端驗證擋下', async () => {
    await card.getByRole('button', { name: '我要送' }).click()
    await expect(dialog.getByText('認領「奶瓶組」')).toBeVisible()
    await dialog.getByRole('button', { name: '確認認領' }).click()
    await expect(dialog).toBeVisible() // 原生 required 阻止送出
    await expect(dialog.getByLabel('你的暱稱（必填）')).toHaveJSProperty('validity.valueMissing', true)
    await dialog.getByLabel('你的暱稱（必填）').fill('   ')
    await dialog.getByRole('button', { name: '確認認領' }).click()
    await expect(dialog.getByText('請填暱稱')).toBeVisible()
  })

  await test.step('填暱稱、數量 2，認領成功', async () => {
    await dialog.getByLabel('你的暱稱（必填）').fill('阿明')
    await dialog.getByRole('button', { name: '增加' }).click()
    await expect(dialog.locator('.g-step b')).toHaveText('2')
    await dialog.getByRole('button', { name: '確認認領' }).click()
    await expect(page.getByRole('heading', { name: '✓ 認領成功！' })).toBeVisible()
    await expect(page.getByText('你認領了 奶瓶組 × 2')).toBeVisible()
    await expect(page.getByText('這份認領已存在此裝置')).toBeVisible()
    await page.getByRole('button', { name: '回到清單繼續看' }).click()
    await expect(card).toContainText('2 / 3')
  })

  await test.step('重新整理後仍顯示「修改我的認領」', async () => {
    await page.reload()
    await expect(card.getByRole('button', { name: '修改我的認領' })).toBeVisible()
    await expect(card).toContainText('你已認領 2 件')
  })

  await test.step('/me/claims 列出這筆認領', async () => {
    await page.getByRole('link', { name: '我的認領' }).click()
    await expect(page).toHaveURL(/\/me\/claims$/)
    await expect(page.getByRole('link', { name: '小愛的待產清單' })).toBeVisible()
    await expect(page.getByText('奶瓶組 × 2')).toBeVisible()
    await expect(page.getByText('阿明')).toBeVisible()
  })

  await test.step('回清單修改數量為 1', async () => {
    await page.getByRole('link', { name: '小愛的待產清單' }).click()
    await card.getByRole('button', { name: '修改我的認領' }).click()
    await expect(dialog.getByText('修改認領「奶瓶組」')).toBeVisible()
    await dialog.getByRole('button', { name: '減少' }).click()
    await dialog.getByRole('button', { name: '儲存修改' }).click()
    await page.getByRole('button', { name: '回到清單繼續看' }).click()
    await expect(card).toContainText('1 / 3')
  })

  await test.step('在 /me/claims 取消，數量釋出', async () => {
    await page.goto('/me/claims')
    await expect(page.getByText('奶瓶組 × 1')).toBeVisible()
    page.once('dialog', d => d.accept())
    await page.getByRole('button', { name: '取消', exact: true }).click()
    await expect(page.getByText('過去的紀錄（1）')).toBeVisible() // 已取消的移到「過去的紀錄」
    await expect(page.getByRole('button', { name: '取消', exact: true })).toHaveCount(0)
    await page.goto(`/s/${list.slug}`)
    await expect(card).toContainText('0 / 3')
    await expect(card.getByRole('button', { name: '我要送' })).toBeVisible()
  })
  await owner.api.dispose()
})
