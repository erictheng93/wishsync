import { chromium, newCtx, registerUser, createList, guestClaim, log, OUT, WEB, pwRequest, sql, sleep } from './lib.mjs'
import { writeFileSync, readFileSync } from 'node:fs'
writeFileSync(OUT + '/03c.jsonl', '')
const L = (tag, o) => { console.log(tag, JSON.stringify(o).slice(0, 700)); log('03c.jsonl', { tag, ...o }) }
const U = await registerUser({ name: '小<愛>"&', tag: 'ssr' })
const T = await createList(U, { title: '特殊 <b>"標題"</b> & 測試', showNames: true, extra: { description: '描述 "引號" <i>x</i> & 換行\n第二行' }, items: [{ title: '品A', qty: 2 }, { title: '品B', qty: 1 }] })
await guestClaim(T.items[0].id, 1, '客X')
const UAS = { line: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Line/13.8.0', safari: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1', linebot: 'facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)', lineCrawler: 'Line/2.0 (crawler)', curl: 'curl/8' }
const api = await pwRequest.newContext()
const meta = (html, p) => html.match(new RegExp(`<meta[^>]+(?:property|name)="${p}"[^>]*?content="([^"]*)"`))?.[1] ?? html.match(new RegExp(`<meta[^>]+content="([^"]*)"[^>]+(?:property|name)="${p}"`))?.[1]
for (const [n, ua] of Object.entries(UAS)) {
  const r = await api.get(`${WEB}/s/${T.slug}`, { headers: { 'User-Agent': ua } }); const h = await r.text()
  L('ssr-' + n, { status: r.status(), h1: /<h1[^>]*>([^<]*)<\/h1>/.exec(h)?.[1], ogTitle: meta(h, 'og:title'), ogDesc: meta(h, 'og:description'), desc: meta(h, 'description'), ogImage: meta(h, 'og:image'), ogUrl: meta(h, 'og:url'), twitter: meta(h, 'twitter:card'), title: /<title>([^<]*)<\/title>/.exec(h)?.[1], cards: (h.match(/class="g-card/g) || []).length, rawUnescapedB: h.includes('<b>"標題"</b>'), cc: r.headers()['cache-control'], vary: r.headers()['vary'] })
}
// 不存在 / 草稿 / 下架 的 meta 與狀態碼
const seed = JSON.parse(readFileSync(OUT + '/seed.json'))
for (const [n, slug] of [['404', 'xxxxxxxxxx'], ['draft', seed.draft.slug], ['hidden', seed.hidden.slug], ['closed', seed.closed.slug]]) { const r = await api.get(`${WEB}/s/${slug}`); const h = await r.text(); L('ssr-state-' + n, { status: r.status(), robots: meta(h, 'robots'), title: /<title>([^<]*)<\/title>/.exec(h)?.[1], leaksTitle: h.includes(n === 'hidden' ? seed.hidden.title : '@@@'), ogTitle: meta(h, 'og:title') }) }
// 無 JS
const browser = await chromium.launch()
const ctx = await browser.newContext({ baseURL: WEB, javaScriptEnabled: false, locale: 'zh-TW' }); const page = await ctx.newPage()
await page.goto(`/s/${T.slug}`); const txt = await page.innerText('body')
L('nojs-share', { hasTitle: txt.includes('標題'), hasItems: txt.includes('品A') && txt.includes('品B'), claimerShown: txt.includes('客X'), buttons: await page.getByRole('button', { name: '我要送' }).count(), enabled: await page.getByRole('button', { name: '我要送' }).first().isEnabled().catch(() => null), tail: txt.slice(-120).replace(/\n/g, '|') })
await page.screenshot({ path: OUT + '/shots/ssr__nojs-share.png' })
for (const p of ['/', '/login', '/terms', '/privacy', '/dashboard', '/me/claims']) { await page.goto(p); L('nojs' + p, { text: (await page.innerText('body').catch(() => '')).slice(0, 100).replace(/\n/g, '|') }) }
await ctx.close()
// LINE UA（有 JS）：提示橫幅
const c2 = await newCtx(browser, { userAgent: UAS.line }); const p2 = await c2.newPage(); await p2.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' })
L('line-ua-banner', { banner: await p2.locator('.g-banner').first().innerText().catch(() => '(none)') })
await p2.screenshot({ path: OUT + '/shots/ssr__line-ua.png' })
// 禁用 localStorage / cookie 的情況（LINE 常見）
const c3 = await newCtx(browser, { userAgent: UAS.line }); await c3.addInitScript(() => { Object.defineProperty(window, 'localStorage', { get() { throw new Error('denied') } }); Object.defineProperty(window, 'sessionStorage', { get() { throw new Error('denied') } }); Object.defineProperty(document, 'cookie', { get() { return '' }, set() {} }) })
const p3 = await c3.newPage(); await p3.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' })
await p3.locator('li.g-card', { hasText: '品B' }).getByRole('button', { name: '我要送' }).click(); await p3.getByLabel('你的暱稱（必填）').fill('無儲存客'); await p3.getByRole('button', { name: '確認認領' }).click(); await p3.waitForTimeout(1500)
L('storage-denied-claim', { done: await p3.getByRole('heading', { name: '✓ 認領成功！' }).isVisible(), banner: (await p3.locator('.g-sheet .g-banner').allInnerTexts()).join('|') })
await p3.screenshot({ path: OUT + '/shots/ssr__storage-denied.png' })
await browser.close()
