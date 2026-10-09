import { chromium, newCtx, watch, AUDIT_FN, registerUser, createList, guestClaim, sessionCookies, sql, q, log, OUT, API, WEB, uid, sleep, randomUUID } from './lib.mjs'
import { writeFileSync, readFileSync, mkdirSync } from 'node:fs'
writeFileSync(OUT + '/02b.jsonl', '')
const seed = JSON.parse(readFileSync(OUT + '/seed.json'))
const X = ['<img src=x onerror=window.__xss=1>', '"><script>window.__xss=2</script>', "';window.__xss=3//", '<svg onload=window.__xss=4>', '{{7*7}} ${7*7}', '<a href="javascript:window.__xss=6">x</a>']
const U = await registerUser({ name: '<img src=x onerror=window.__xss=9>', tag: 'xss' })
const L = await createList(U, { title: X[0], showNames: true, extra: { description: X[1] }, items: X.map((x, i) => ({ title: x, qty: 5, extra: { brand: X[(i + 1) % 6], spec: X[(i + 2) % 6], description: x, product_url: i === 1 ? 'https://example.com/?a="><script>window.__xss=7</script>' : 'https://example.com/' + i } })) })
const longL = await createList(U, { title: 'W'.repeat(100), showNames: true, extra: { description: '很長的描述 ' + 'D'.repeat(300) }, items: [{ title: 'I'.repeat(120), qty: 3, extra: { brand: 'B'.repeat(100), spec: 'S'.repeat(200) } }, { title: '一二三四五六七八九十'.repeat(12), qty: 1 }] })
const claimers = []
for (let i = 0; i < 6; i++) { const g = await guestClaim(L.items[i].id, 1, ['<b>x</b>', '"><script>window.__xss=12</script>', "';window.__xss=13//", '<svg onload=window.__xss=14>', 'N'.repeat(30), '👨‍👩‍👧‍👦'.repeat(4)][i], { note: X[i], contact: X[(i + 3) % 6] }); claimers.push(g) }
await guestClaim(longL.items[0].items?.[0] ?? longL.items[0].id, 1, '長'.repeat(30), { note: 'n'.repeat(200), contact: 'c'.repeat(100) })
const browser = await chromium.launch()
const hits = []
async function visit(role, ctxOpts, path, tag, vp = { width: 390, height: 844 }) {
  const ctx = await newCtx(browser, { viewport: vp, ...ctxOpts })
  const page = await ctx.newPage(); const sink = []; watch(page, tag, sink)
  const dialogs = []; page.on('dialog', d => { dialogs.push(d.message()); d.dismiss() })
  await page.goto(path, { waitUntil: 'networkidle' }); await page.waitForTimeout(500)
  const x = await page.evaluate(() => ({ xss: window.__xss ?? null, injected: [...document.querySelectorAll('img[src="x"], svg[onload], script:not([src]):not([type])')].filter(e => !e.closest('#__nuxt') ? false : true).map(e => e.outerHTML.slice(0, 80)), jsLinks: [...document.querySelectorAll('a[href^="javascript:" i]')].map(a => a.outerHTML.slice(0, 80)), text: document.body.innerText.slice(0, 300), tpl: /\b49\b/.test(document.body.innerText) }))
  const audit = await page.evaluate(AUDIT_FN)
  await page.screenshot({ path: `${OUT}/shots/in__${tag}.png` })
  log('02b.jsonl', { tag, dialogs, ...x, text: undefined, overflowX: audit.overflowX, trunc: audit.truncated.slice(0, 5), culprits: audit.overflowCulprits })
  await ctx.close()
}
const A = (c) => ({ cookies: c })
await visit('anon', {}, `/s/${L.slug}`, 'xss-share')
await visit('guest', { cookies: [{ name: 'ws_guest', value: claimers[0].token, url: API }] }, `/s/${L.slug}`, 'xss-share-guest')
await visit('creator', A(await sessionCookies(U)), '/dashboard', 'xss-dashboard')
await visit('creator', A(await sessionCookies(U)), `/lists/${L.id}/edit`, 'xss-edit')
await visit('creator', A(await sessionCookies(U)), `/lists/${L.id}/progress`, 'xss-progress')
await visit('creator', A(await sessionCookies(U)), `/settings`, 'xss-settings')
await visit('creator', A(await sessionCookies(U)), `/s/${L.slug}`, 'xss-share-own')
{ const ctx = await newCtx(browser, { cookies: [{ name: 'ws_guest', value: claimers[0].token, url: API }] }); const p = await ctx.newPage(); await p.addInitScript(t => localStorage.setItem('ws_guest_token', t), claimers[0].token); const d = []; p.on('dialog', x => { d.push(x.message()); x.dismiss() }); await p.goto('/me/claims', { waitUntil: 'networkidle' }); await p.waitForTimeout(500); log('02b.jsonl', { tag: 'xss-me-claims', dialogs: d, xss: await p.evaluate(() => window.__xss ?? null), text: (await p.innerText('body')).slice(0, 200) }); await ctx.close() }
// staff: admin lists + reports (report detail with xss)
sql(`update users set is_staff=true where email='${q(seed.S.email)}'`)
{ const ga = await (await import('./lib.mjs')).guestApi(); await ga.post(`public/wishlists/${L.slug}/reports`, { data: { reason: 'other', detail: X[0] + X[1], item_id: null, turnstile_token: null } }) }
for (const vp of [{ width: 320, height: 568 }, { width: 1440, height: 900 }]) {
  for (const [name, click] of [['reports', null], ['lists', '清單搜尋'], ['users', '使用者']]) {
    const ctx = await newCtx(browser, { cookies: seed.S.cookies, viewport: vp }); const page = await ctx.newPage(); const d = []; page.on('dialog', x => { d.push(x.message()); x.dismiss() })
    await page.goto('/admin', { waitUntil: 'networkidle' })
    if (click) { await page.getByRole('button', { name: click }).click(); await page.waitForTimeout(300); if (name === 'lists') { await page.getByLabel('搜尋清單').fill(L.slug.slice(0, 4)); await page.keyboard.press('Enter') } if (name === 'users') { await page.getByLabel('搜尋使用者').fill('qa-xss'); await page.keyboard.press('Enter') } await page.waitForTimeout(700) }
    const a = await page.evaluate(AUDIT_FN)
    log('02b.jsonl', { tag: `xss-admin-${name}-${vp.width}`, dialogs: d, xss: await page.evaluate(() => window.__xss ?? null), overflowX: a.overflowX, trunc: a.truncated.slice(0, 5) })
    await page.screenshot({ path: `${OUT}/shots/in__admin-${name}-${vp.width}.png` }); await ctx.close()
  }
}
// 長文字版面
for (const vp of [{ width: 320, height: 568 }, { width: 1440, height: 900 }]) {
  await visit('anon', {}, `/s/${longL.slug}`, `long-share-${vp.width}`, vp)
  await visit('creator', A(await sessionCookies(U)), '/dashboard', `long-dashboard-${vp.width}`, vp)
  await visit('creator', A(await sessionCookies(U)), `/lists/${longL.id}/edit`, `long-edit-${vp.width}`, vp)
  await visit('creator', A(await sessionCookies(U)), `/lists/${longL.id}/progress`, `long-progress-${vp.width}`, vp)
}
// ---- 互動：暱稱 31-40 字、貼上換行、雙擊、離線送出 ----
const T = await createList(U, { title: '互動測試', items: [{ title: '雙擊品項', qty: 5 }, { title: '離線品項', qty: 5 }, { title: '長暱稱品項', qty: 5 }, { title: '貼上品項', qty: 5 }] })
const open = async (page, name) => { await page.locator('li.g-card', { hasText: name }).getByRole('button', { name: '我要送' }).click() }
{ // 31-40 字暱稱
  const ctx = await newCtx(browser); const page = await ctx.newPage(); await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' })
  await open(page, '長暱稱品項'); const d = page.getByRole('dialog'); await d.getByLabel('你的暱稱（必填）').fill('あ'.repeat(35)); await d.getByRole('button', { name: '確認認領' }).click(); await page.waitForTimeout(800)
  log('02b.jsonl', { tag: 'nick35', sheetText: (await d.innerText().catch(() => '')).slice(0, 300), inputMaxlength: await d.getByLabel('你的暱稱（必填）').getAttribute('maxlength').catch(() => null) })
  await page.screenshot({ path: `${OUT}/shots/in__nick35.png` }); await ctx.close()
}
{ // 貼上含換行
  const ctx = await newCtx(browser); const page = await ctx.newPage(); await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' })
  await open(page, '貼上品項'); const d = page.getByRole('dialog')
  const f = d.getByLabel('你的暱稱（必填）'); await f.focus()
  await page.evaluate(() => { const e = document.activeElement; const dt = new DataTransfer(); dt.setData('text/plain', '阿明\n第二行\r\n第三行'); e.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true })); })
  await page.keyboard.insertText('阿明\n第二行'); const v = await f.inputValue()
  await d.getByLabel('留言（選填）').fill(''); await d.getByLabel('留言（選填）').focus(); await page.keyboard.insertText('a\nb\nc'); const nv = await d.getByLabel('留言（選填）').inputValue()
  log('02b.jsonl', { tag: 'paste-newline', nick: JSON.stringify(v), note: JSON.stringify(nv) }); await ctx.close()
}
{ // 雙擊送出
  const ctx = await newCtx(browser); const page = await ctx.newPage(); const posts = []
  page.on('request', r => { if (r.method() === 'POST' && /claims/.test(r.url())) posts.push(r.headers()['idempotency-key']) })
  await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' }); await open(page, '雙擊品項'); const d = page.getByRole('dialog')
  await d.getByLabel('你的暱稱（必填）').fill('雙擊哥'); await d.getByRole('button', { name: '確認認領' }).dblclick(); await page.waitForTimeout(1500)
  const n = sql(`select count(*) from claims c join wishlist_items i on i.id=c.item_id where i.id='${T.items[0].id}'`)
  log('02b.jsonl', { tag: 'dblclick-claim', posts: posts.length, distinctKeys: new Set(posts).size, claimsInDb: n, success: await page.getByRole('heading', { name: '✓ 認領成功！' }).isVisible() })
  // 雙擊「我要送」
  await ctx.close()
}
{ // 離線送出
  const ctx = await newCtx(browser); const page = await ctx.newPage(); const posts = []
  page.on('request', r => { if (r.method() === 'POST' && /claims/.test(r.url())) posts.push(r.headers()['idempotency-key']) })
  await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' }); await open(page, '離線品項'); const d = page.getByRole('dialog')
  await d.getByLabel('你的暱稱（必填）').fill('離線哥')
  await ctx.setOffline(true); await d.getByRole('button', { name: '確認認領' }).click(); await page.waitForTimeout(1200)
  const offlineText = await page.locator('body').innerText()
  const msg = await d.locator('.g-banner.err').innerText().catch(() => '(none)')
  const btnDisabled = await d.getByRole('button', { name: /確認認領|離線中|送出中/ }).first().innerText().catch(() => '?')
  await page.screenshot({ path: `${OUT}/shots/in__offline-submit.png` })
  await ctx.setOffline(false); await page.waitForTimeout(800)
  await d.getByRole('button', { name: /確認認領/ }).click().catch(e => log('02b.jsonl', { tag: 'offline-retry-click-fail', e: String(e).slice(0, 100) }))
  await page.waitForTimeout(1500)
  log('02b.jsonl', { tag: 'offline-submit', msg, btn: btnDisabled, posts, success: await page.getByRole('heading', { name: '✓ 認領成功！' }).isVisible(), claimsInDb: sql(`select count(*) from claims where item_id='${T.items[1].id}'`) })
  await ctx.close()
}
await browser.close()
console.log('done', L.slug, longL.slug)
