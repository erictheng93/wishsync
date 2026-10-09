import { expect, test } from '@playwright/test'

// PWA（F-22）：manifest、Service Worker 安裝/範圍、快取內容、離線導覽。
// 此 spec 不依賴 API；用 /login（CSR 頁）當入口。Service Worker 僅在 chromium 且 production build 可用。
test.use({ serviceWorkers: 'allow' })

async function swReady(page: import('@playwright/test').Page) {
  await page.goto('/login')
  await page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined))
  // 首次載入 SW 尚未控制頁面（不 skipWaiting/claim，刻意設計）；重新載入後才受控
  await page.reload()
  await expect.poll(() => page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true)
}

test('/offline 回 200 且是離線頁', async ({ request }) => {
  const r = await request.get('/offline')
  expect(r.status()).toBe(200)
  expect(r.headers()['content-type']).toContain('text/html')
  expect(await r.text()).toContain('目前離線')
})

test('manifest 可解析、圖示皆 200', async ({ request, page }) => {
  await page.goto('/login')
  const href = await page.locator('link[rel=manifest]').getAttribute('href')
  const res = await request.get(href!)
  expect(res.status()).toBe(200)
  const m = await res.json()
  expect(m).toMatchObject({ scope: '/', display: 'standalone' })
  expect(m.icons.length).toBeGreaterThan(0)
  for (const ic of m.icons) {
    const r = await request.get(ic.src)
    expect(r.status(), ic.src).toBe(200)
    expect(r.headers()['content-type']).toContain('image/png')
  }
})

test('SW 註冊、scope 為 /，快取只含離線頁與靜態資產', async ({ page }) => {
  await swReady(page)
  const reg = await page.evaluate(async () => {
    const r = await navigator.serviceWorker.getRegistration()
    return { scope: r?.scope, active: r?.active?.state }
  })
  expect(new URL(reg.scope!).pathname).toBe('/')
  expect(reg.active).toBe('activated')

  // 再走訪幾個頁面（含 /s/*、API 呼叫），確保它們不會被寫入快取
  await page.goto('/s/does-not-exist').catch(() => {})
  await page.goto('/register')
  await page.evaluate(() => fetch('/api/v1/me').catch(() => {}))

  const keys = await page.evaluate(async () => {
    const out: string[] = []
    for (const n of await caches.keys()) for (const r of await (await caches.open(n)).keys()) out.push(new URL(r.url).pathname)
    return out
  })
  expect(keys).toContain('/offline')
  expect(keys).toContain('/icons/icon-192.png')
  for (const p of keys) {
    expect(p, '不得快取分享頁').not.toMatch(/^\/s\//)
    expect(p, '不得快取 API').not.toMatch(/^\/api\//)
    expect(p, '不得快取 HTML 導覽').not.toMatch(/^\/(login|register|dashboard|lists|settings|admin|forgot-password)?$/)
  }
  // 只允許：/offline、/icons/*、/_nuxt/*（不含 /_nuxt/builds/*）
  for (const p of keys) expect(p).toMatch(/^\/(offline|icons\/.+|_nuxt\/(?!builds\/).+)$/)
})

// 限制：context.setOffline 讓「網路」失敗，但 SW 本身仍在；等同真實離線時 fetch() 拒絕的情況，足以驗證 fallback。
test('離線時導覽顯示離線頁，恢復連線後可正常載入', async ({ page, context }) => {
  await swReady(page)
  await context.setOffline(true)
  await page.goto('/register').catch(() => {})
  await expect(page.getByRole('heading', { name: '目前離線' })).toBeVisible()
  await context.setOffline(false)
  // 離線頁監聽 online 事件並自動重新整理，所以不另外 goto（會與自動重載競態）
  await expect(page.getByRole('heading', { name: '目前離線' })).toBeHidden()
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
})
