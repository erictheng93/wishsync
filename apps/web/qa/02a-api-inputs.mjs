// 輸入探索（API 層）：邊界、惡意字串、數值、日期、header/body。結果 out/02a.jsonl
import { registerUser, createList, guestApi, guestClaim, randomUUID, sql, q, log, OUT, API, WEB, pwRequest, uid } from './lib.mjs'
import { writeFileSync, readFileSync } from 'node:fs'
writeFileSync(OUT + '/02a.jsonl', '')
const U = await registerUser({ name: '輸入測試者', tag: 'in' })
const base = await createList(U, { title: '輸入測試基底', items: [{ title: '基底品項', qty: 50 }], showNames: true })
const R = (name, status, extra = {}) => { log('02a.jsonl', { name, status, ...extra }); return status }
const P = {
  empty: '', spaces: '   ', nbsp: '   ', zwsp: '​​​', emoji: '🎁🎉👨‍👩‍👧‍👦🏳️‍🌈', rtl: 'مرحبا بالعالم ‮evil‬ עברית', bidiOverride: 'abc‮def',
  nul: 'a\u0000b', bel: 'a\u0007b', esc: 'a\u001bb[31m', nl: 'line1\nline2\r\nline3', tab: 'a\tb', crlfHdr: 'Hi\r\nBcc: evil@example.com', lone: 'a\ud800b',
  l30: 'あ'.repeat(30), l31: 'あ'.repeat(31), l100: 'x'.repeat(100), l101: 'x'.repeat(101), l120: 'x'.repeat(120), l121: 'x'.repeat(121), l2000: 'x'.repeat(2000), l10000: 'x'.repeat(10000),
  xss1: '<img src=x onerror=window.__xss=1>', xss2: '"><script>window.__xss=2</script>', xss3: "';window.__xss=3//", xss4: '<svg onload=window.__xss=4>', xss5: '{{7*7}} ${7*7} #{7*7}', xss6: '<a href="javascript:window.__xss=6">x</a>',
  sql1: "'; DROP TABLE users;--", sql2: "' OR '1'='1", sql3: '\\', sql4: '%_', sql5: "Robert'); DROP TABLE claims;--",
}
const res = {}
// 清單標題
for (const [k, v] of Object.entries(P)) {
  const r = await U.api.post('wishlists', { data: { type: 'registry', title: v, visibility: 'link' } })
  res['title:' + k] = r.status(); R('list.title:' + k, r.status(), { body: (await r.text()).slice(0, 160) })
}
// 品項欄位
for (const [k, v] of Object.entries(P)) for (const f of ['title', 'description', 'brand', 'spec']) {
  const r = await U.api.post(`wishlists/${base.id}/items`, { data: { title: f === 'title' ? v : '一般品項', [f]: v, qty_needed: 1 } })
  R(`item.${f}:${k}`, r.status(), { body: r.ok() ? '' : (await r.text()).slice(0, 120) })
}
// product_url
const urls = { js: 'javascript:alert(1)', data: 'data:text/html,<script>alert(1)</script>', ftp: 'ftp://example.com/x', upper: 'HTTP://EXAMPLE.COM/A', noscheme: 'example.com', space: 'https://exa mple.com', cred: 'https://good.com@evil.com/', long2000: 'https://example.com/' + 'a'.repeat(1980), long2001: 'https://example.com/' + 'a'.repeat(1981), emptyHost: 'http:///x', file: 'file:///etc/passwd', xssq: 'https://example.com/?a="><script>window.__xss=7</script>', unicode: 'https://例え.jp/パス', tab: 'java\tscript:alert(1)', jsMixed: ' JaVaScRiPt:alert(1)', http: 'http://localhost:8082/api/v1/me' }
for (const [k, v] of Object.entries(urls)) {
  const r = await U.api.post(`wishlists/${base.id}/items`, { data: { title: 'url:' + k, product_url: v, qty_needed: 1 } })
  R('product_url:' + k, r.status(), { body: r.ok() ? JSON.stringify((await r.json()).product_url).slice(0, 80) : (await r.text()).slice(0, 100) })
}
// 數值
const nums = { zero: 0, neg: -1, frac: 1.5, big: 1e9, over: 10000, max: 9999, str: '3', nul: null, huge: 9999999999999, bool: true, exp: 1e2 }
for (const [k, v] of Object.entries(nums)) {
  const r = await U.api.post(`wishlists/${base.id}/items`, { data: { title: 'qn:' + k, qty_needed: v } })
  R('qty_needed:' + k, r.status(), { body: r.ok() ? 'ok qty=' + (await r.json()).qty_needed : (await r.text()).slice(0, 100) })
}
for (const [k, v] of Object.entries({ neg: -5, frac: 9.99, big: 1e12, huge: 1e19, str: '100', zero: 0 })) {
  const r = await U.api.post(`wishlists/${base.id}/items`, { data: { title: 'price:' + k, unit_price_amount: v, qty_needed: 1 } })
  R('unit_price:' + k, r.status(), { body: r.ok() ? 'ok' : (await r.text()).slice(0, 100) })
}
// 認領數量
const item = base.items[0]
for (const [k, v] of Object.entries({ zero: 0, neg: -1, frac: 1.5, str: '2', max99: 99, over100: 100, big: 2147483648, nul: null, miss: undefined })) {
  const r = await (await guestApi()).post(`items/${item.id}/claims`, { data: { qty: v, display_name: 'Q' + k }, headers: { 'Idempotency-Key': randomUUID() } })
  R('claim.qty:' + k, r.status(), { body: (await r.text()).slice(0, 140) })
}
// 認領欄位（暱稱/備註/聯絡/email）
const f2 = await createList(U, { title: '認領欄位測試', items: Array.from({ length: 1 }, () => ({ title: '品', qty: 500 })), showNames: true })
for (const [k, v] of Object.entries(P)) {
  for (const f of ['display_name', 'note', 'contact']) {
    const data = { qty: 1, display_name: '訪客', [f]: v }
    const r = await (await guestApi()).post(`items/${f2.items[0].id}/claims`, { data, headers: { 'Idempotency-Key': randomUUID() } })
    R(`claim.${f}:${k}`, r.status(), { body: r.ok() ? '' : (await r.text()).slice(0, 120) })
  }
}
for (const [k, v] of Object.entries({ noat: 'abc', long: 'a'.repeat(190) + '@x.com', unicode: '用戶@例え.jp', crlf: 'a@b.com\r\nBcc:x@y.com', space: 'a b@c.com', xss: '<script>@x.com', upper: 'A@B.COM' })) {
  const r = await (await guestApi()).post(`items/${f2.items[0].id}/claims`, { data: { qty: 1, display_name: 'E' + k, email: v }, headers: { 'Idempotency-Key': randomUUID() } })
  R('claim.email:' + k, r.status(), { body: r.ok() ? '' : (await r.text()).slice(0, 120) })
}
// 日期
for (const [k, v] of Object.entries({ feb30: '2026-02-30', leap: '2028-02-29', nonleap: '2027-02-29', zero: '0000-00-00', y0: '0000-01-01', y9999: '9999-12-31', y10000: '10000-01-01', today: new Date(Date.now() + 8 * 36e5).toISOString().slice(0, 10), yest: new Date(Date.now() + 8 * 36e5 - 864e5).toISOString().slice(0, 10), iso: '2027-01-01T00:00:00Z', slash: '2027/01/01', neg: '-0001-01-01', num: 20270101 })) {
  const r = await U.api.post('wishlists', { data: { type: 'registry', title: 'd:' + k, event_date: v } })
  const r2 = await U.api.post('wishlists', { data: { type: 'registry', title: 'ds:' + k, event_date: v, surprise_mode: true } })
  R('event_date:' + k, r.status(), { surprise: r2.status(), body: r.ok() ? JSON.stringify((await r.json()).event_date) : (await r.text()).slice(0, 100) })
}
for (const [k, v] of Object.entries({ zero: 0, neg: -1, one: 1, y: 8760, over: 8761, huge: 1e10, frac: 1.5, str: '5' })) {
  const r = await U.api.post('wishlists', { data: { type: 'registry', title: 'ttl:' + k, claim_ttl_hours: v } })
  R('claim_ttl_hours:' + k, r.status(), { body: r.ok() ? '' : (await r.text()).slice(0, 100) })
}
// 請求格式
const raw = async (name, method, path, { body, headers = {}, cookies = true } = {}) => {
  const ctx = await pwRequest.newContext({ baseURL: API + '/api/v1/', extraHTTPHeaders: { Origin: WEB, ...(cookies ? { Cookie: (await U.api.storageState()).cookies.map(c => `${c.name}=${c.value}`).join('; ') } : {}) } })
  const t0 = Date.now()
  try {
    const r = await ctx.fetch(path, { method, data: body, headers, timeout: 60000 })
    const txt = await r.text()
    R(name, r.status(), { ms: Date.now() - t0, ct: r.headers()['content-type'], body: txt.slice(0, 200) })
    return r
  } catch (e) { R(name, 'ERR', { err: String(e).slice(0, 150) }) } finally { await ctx.dispose() }
}
await raw('body.invalidjson', 'POST', 'wishlists', { body: '{bad', headers: { 'Content-Type': 'application/json' } })
await raw('body.array', 'POST', 'wishlists', { body: '[]', headers: { 'Content-Type': 'application/json' } })
await raw('body.nocontenttype', 'POST', 'wishlists', { body: '{"type":"registry","title":"x"}', headers: { 'Content-Type': 'text/plain' } })
await raw('body.empty', 'POST', 'wishlists', { body: '', headers: { 'Content-Type': 'application/json' } })
await raw('body.2MB', 'POST', 'wishlists', { body: JSON.stringify({ type: 'registry', title: 'x', description: 'y'.repeat(2 * 1024 * 1024) }), headers: { 'Content-Type': 'application/json' } })
await raw('body.20MB', 'POST', 'wishlists', { body: JSON.stringify({ type: 'registry', title: 'x', description: 'y'.repeat(20 * 1024 * 1024) }), headers: { 'Content-Type': 'application/json' } })
await raw('body.deepjson', 'POST', 'wishlists', { body: '['.repeat(100000) + ']'.repeat(100000), headers: { 'Content-Type': 'application/json' } })
await raw('claim.body.20MB', 'POST', `items/${item.id}/claims`, { cookies: false, body: JSON.stringify({ qty: 1, display_name: 'x', note: 'y'.repeat(20 * 1024 * 1024) }), headers: { 'Content-Type': 'application/json', 'Idempotency-Key': randomUUID() } })
await raw('claim.noIdemKey', 'POST', `items/${item.id}/claims`, { cookies: false, body: { qty: 1, display_name: 'x' } })
await raw('claim.badIdemKey', 'POST', `items/${item.id}/claims`, { cookies: false, body: { qty: 1, display_name: 'x' }, headers: { 'Idempotency-Key': 'not-a-uuid' } })
await raw('hdr.big', 'GET', 'me', { headers: { 'X-Big': 'a'.repeat(70000) } })
await raw('hdr.hugeCookie', 'GET', 'me', { headers: { Cookie: 'ws_session=' + 'a'.repeat(100000) } })
await raw('hdr.guestTokenLong', 'GET', 'guest/me', { cookies: false, headers: { 'X-Guest-Token': 'a'.repeat(60000) } })
await raw('path.longslug', 'GET', 'public/wishlists/' + 'a'.repeat(5000), { cookies: false })
await raw('path.badUuid', 'GET', 'wishlists/not-a-uuid')
await raw('path.unicode', 'GET', 'public/wishlists/%E4%B8%AD%E6%96%87%E4%B8%AD%E6%96%87%E4%B8%AD%E6%96%87', { cookies: false })
await raw('path.nul', 'GET', 'public/wishlists/abc%00defghi', { cookies: false })
await raw('query.cursorBad', 'GET', 'wishlists?cursor=%ff%ff', {})
await raw('query.limit0', 'GET', 'wishlists?limit=0', {})
await raw('query.limit-1', 'GET', 'wishlists?limit=-1', {})
await raw('query.limitHuge', 'GET', 'wishlists?limit=99999999999999999999', {})
await raw('method.TRACE', 'TRACE', 'me', {})
await raw('method.PUT-me', 'PUT', 'me', {})
await raw('api.unknown', 'GET', 'nope', {})
console.log('done')
