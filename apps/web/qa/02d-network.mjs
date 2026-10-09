import { chromium, newCtx, registerUser, createList, sql, log, OUT } from './lib.mjs'
import { writeFileSync as w } from 'node:fs'
w(OUT + '/02d.jsonl', '')
const U = await registerUser({ tag: 'net' })
const T = await createList(U, { title: '網路中斷測試', items: [{ title: '中斷品項', qty: 5 }, { title: '回應遺失品項', qty: 5 }, { title: '慢速品項', qty: 5 }] })
const browser = await chromium.launch()
const open = async (page, name) => page.locator('li.g-card', { hasText: name }).getByRole('button', { name: '我要送' }).click()
// A. 請求送出前失敗（fetch 直接失敗，navigator.onLine 仍為 true）
{
  const ctx = await newCtx(browser); const page = await ctx.newPage(); const posts = []
  page.on('request', r => { if (r.method() === 'POST' && /claims/.test(r.url())) posts.push(r.headers()['idempotency-key']) })
  await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' }); await open(page, '中斷品項'); const d = page.getByRole('dialog')
  await d.getByLabel('你的暱稱（必填）').fill('斷網哥')
  await page.route('**/items/*/claims', r => r.abort('failed'))
  await d.getByRole('button', { name: '確認認領' }).click(); await page.waitForTimeout(1000)
  const msg = await d.locator('.g-banner.err').innerText().catch(() => '(none)')
  const btn = await d.locator('button.g-btn').innerText()
  await page.screenshot({ path: `${OUT}/shots/net__abort.png` })
  await page.unroute('**/items/*/claims'); await d.getByRole('button', { name: '確認認領' }).click(); await page.waitForTimeout(1500)
  log('02d.jsonl', { tag: 'abort-before-send', msg, btn, posts, keysSame: new Set(posts).size === 1, success: await page.getByRole('heading', { name: '✓ 認領成功！' }).isVisible(), claims: sql(`select count(*) from claims where item_id='${T.items[0].id}'`) })
  await ctx.close()
}
// B. 伺服器已處理但回應遺失（continue 後 abort）→ 重送同 key；匿名首次認領的 guest_token 因此遺失？
{
  const ctx = await newCtx(browser); const page = await ctx.newPage(); const posts = []
  page.on('request', r => { if (r.method() === 'POST' && /claims/.test(r.url())) posts.push(r.headers()['idempotency-key']) })
  await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' }); await open(page, '回應遺失品項'); const d = page.getByRole('dialog')
  await d.getByLabel('你的暱稱（必填）').fill('遺失哥')
  await page.route('**/items/*/claims', async r => { await r.fetch(); await r.abort('failed') })
  await d.getByRole('button', { name: '確認認領' }).click(); await page.waitForTimeout(1200)
  const msg = await d.locator('.g-banner.err').innerText().catch(() => '(none)')
  await page.unroute('**/items/*/claims'); await d.getByRole('button', { name: '確認認領' }).click(); await page.waitForTimeout(1500)
  const ok = await page.getByRole('heading', { name: '✓ 認領成功！' }).isVisible()
  const bodyText = await page.locator('body').innerText()
  const tokenStored = await page.evaluate(() => localStorage.getItem('ws_guest_token'))
  await page.screenshot({ path: `${OUT}/shots/net__lostresp.png` })
  log('02d.jsonl', { tag: 'lost-response-retry', firstMsg: msg, posts, success: ok, tokenStored: !!tokenStored, claimsInDb: sql(`select count(*) from claims where item_id='${T.items[1].id}'`), guestsNamed: sql(`select count(*) from guests where display_name='遺失哥'`), snippet: bodyText.slice(0, 200).replace(/\n/g, '|') })
  await ctx.close()
}
// C. 離線事件：sheet 內按鈕狀態
{
  const ctx = await newCtx(browser); const page = await ctx.newPage()
  await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' }); await open(page, '慢速品項'); const d = page.getByRole('dialog')
  await ctx.setOffline(true); await page.waitForTimeout(500)
  log('02d.jsonl', { tag: 'offline-event-sheet', btn: await d.locator('button.g-btn').innerText(), banner: await page.locator('.g-banner').first().innerText().catch(() => '(none)') })
  await ctx.setOffline(false); await page.waitForTimeout(500)
  log('02d.jsonl', { tag: 'online-again', btn: await d.locator('button.g-btn').innerText() })
  await ctx.close()
}
await browser.close()
