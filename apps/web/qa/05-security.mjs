// 本機自建環境安全煙霧測試（只打 localhost:8082）
import { registerUser, createList, guestClaim, guestApi, sessionCookies, sql, q, log, OUT, API, WEB, randomUUID, pwRequest, PASSWORD, apiCtx } from './lib.mjs'
import { writeFileSync, readFileSync } from 'node:fs'
writeFileSync(OUT + '/05.jsonl', '')
const L = (tag, o) => { console.log(tag, JSON.stringify(o).slice(0, 500)); log('05.jsonl', { tag, ...o }) }
const seed = JSON.parse(readFileSync(OUT + '/seed.json'))
sql('delete from rate_limits')
const A = await registerUser({ tag: 'sa' }), B = await registerUser({ tag: 'sb' })
const LA = await createList(A, { title: 'A 的清單', items: [{ title: 'A品', qty: 5 }], showNames: true }), LB = await createList(B, { title: 'B 的清單', items: [{ title: 'B品', qty: 5 }] })
const gB = await guestClaim(LB.items[0].id, 1, 'B客', { email: 'bguest@example.com' })
const gA = await guestClaim(LA.items[0].id, 1, 'A客')
const st = async (p) => (await p).status()
// --- IDOR ---
const idor = {}
idor['A GET B wishlist'] = await st(A.api.get(`wishlists/${LB.id}`))
idor['A PATCH B wishlist'] = await st(A.api.patch(`wishlists/${LB.id}`, { data: { title: 'hacked' } }))
idor['A DELETE B wishlist'] = await st(A.api.delete(`wishlists/${LB.id}`))
idor['A GET B dashboard'] = await st(A.api.get(`wishlists/${LB.id}/dashboard`))
idor['A POST B item'] = await st(A.api.post(`wishlists/${LB.id}/items`, { data: { title: 'x' } }))
idor['A PATCH B item'] = await st(A.api.patch(`items/${LB.items[0].id}`, { data: { title: 'hacked' } }))
idor['A DELETE B item'] = await st(A.api.delete(`items/${LB.items[0].id}?force=true`))
idor['A reorder B'] = await st(A.api.post(`wishlists/${LB.id}/items/reorder`, { data: { item_ids: [LB.items[0].id] } }))
idor['A (non-owner user) PATCH B guest claim'] = await st(A.api.patch(`claims/${gB.claimId}`, { data: { qty: 2 } }))
idor['A (non-owner user) DELETE B guest claim'] = await st(A.api.delete(`claims/${gB.claimId}`))
idor['B owner PATCH own list guest claim qty'] = await st(B.api.patch(`claims/${gB.claimId}`, { data: { qty: 3 } }))
const gc = await guestApi()
idor['guest A token PATCH B claim'] = await st(gc.patch(`claims/${gB.claimId}`, { data: { qty: 2 }, headers: { 'X-Guest-Token': gA.token } }))
idor['guest A token DELETE B claim'] = await st(gc.delete(`claims/${gB.claimId}`, { headers: { 'X-Guest-Token': gA.token } }))
idor['anon DELETE claim'] = await st(gc.delete(`claims/${gB.claimId}`))
const meB = await (await gc.get('guest/me', { headers: { 'X-Guest-Token': gA.token } })).json(); idor['guest A /guest/me lists only own'] = JSON.stringify(meB.claims.map(c => c.claim.claimer_name))
const ex = await (await A.api.get('me/export')).text(); idor['A export leaks B'] = ex.includes('B客') || ex.includes('bguest@example.com') || ex.includes(LB.slug)
idor['A export contains A guest claimer (owner sees)'] = ex.includes('A客')
idor['non-staff admin/stats'] = await st(A.api.get('admin/stats')); idor['non-staff admin/users'] = await st(A.api.get('admin/users')); idor['non-staff moderate'] = await st(A.api.patch(`admin/wishlists/${LB.id}/moderation`, { data: { moderation_status: 'hidden', reason: 'x' } })); idor['non-staff flags put'] = await st(A.api.put('admin/system-flags/read_only', { data: { value: true } })); idor['non-staff admin/reports'] = await st(A.api.get('admin/reports'))
idor['uploads presign foreign list'] = (await A.api.post('uploads/presign', { data: { purpose: 'item', content_type: 'image/png', size: 1000 } })).status()
const nc = await pwRequest.newContext({ baseURL: API + '/api/v1/', extraHTTPHeaders: { Origin: WEB } })
for (const [m, p] of [['get', 'wishlists'], ['post', 'wishlists'], ['get', `wishlists/${LA.id}`], ['get', 'me'], ['get', 'me/export'], ['delete', 'me'], ['patch', 'me'], ['get', 'admin/stats'], ['post', 'uploads/presign'], ['get', 'guest/me'], ['post', 'auth/logout']]) idor['anon ' + m.toUpperCase() + ' ' + p.replace(LA.id, '{id}')] = await st(nc[m](p, m === 'post' || m === 'patch' ? { data: {} } : {}))
L('idor', idor)
// --- CORS ---
const cors = {}
const raw = await pwRequest.newContext()
for (const origin of ['https://evil.example.com', 'http://localhost:3014', 'http://localhost:3014.evil.com', 'null', 'http://localhost:3015']) {
  const pre = await raw.fetch(API + '/api/v1/wishlists', { method: 'OPTIONS', headers: { Origin: origin, 'Access-Control-Request-Method': 'POST', 'Access-Control-Request-Headers': 'content-type' } })
  const get = await raw.get(API + '/api/v1/public/wishlists/' + LA.slug, { headers: { Origin: origin } })
  cors[origin] = { preflight: pre.status(), acao: pre.headers()['access-control-allow-origin'] ?? null, acac: pre.headers()['access-control-allow-credentials'] ?? null, getAcao: get.headers()['access-control-allow-origin'] ?? null, vary: get.headers()['vary'] }
}
L('cors', cors)
// CSRF 型：跨站、無 preflight 的簡單請求（POST 無 body / DELETE 屬非簡單）
const sess = (await A.api.storageState()).cookies.find(c => c.name === 'ws_session')
const csrf = await raw.post(API + '/api/v1/auth/logout', { headers: { Origin: 'https://evil.example.com', Cookie: `ws_session=${sess.value}` } })
L('csrf-logout-with-evil-origin', { status: csrf.status(), note: '伺服器不檢查 Origin；是否可被跨站利用取決於 SameSite=Lax（跨站 POST 瀏覽器不帶 cookie）' })
// --- cookie 屬性 ---
const lg = await raw.post(API + '/api/v1/auth/login', { data: { email: A.email, password: PASSWORD }, headers: { Origin: WEB } })
L('cookie-attrs-login', { setCookie: lg.headersArray().filter(h => /set-cookie/i.test(h.name)).map(h => h.value.replace(/=[^;]+/, '=<redacted>')) })
const gcl = await raw.post(API + `/api/v1/items/${LA.items[0].id}/claims`, { data: { qty: 1, display_name: 'ck' }, headers: { Origin: WEB, 'Idempotency-Key': randomUUID(), 'Content-Type': 'application/json' } })
L('cookie-attrs-guest', { setCookie: gcl.headersArray().filter(h => /set-cookie/i.test(h.name)).map(h => h.value.replace(/=[^;]+/, '=<redacted>')), cacheControl: gcl.headers()['cache-control'] })
// 響應標頭
const h = await raw.get(API + '/api/v1/me'); L('security-headers-api', Object.fromEntries(Object.entries(h.headers()).filter(([k]) => /x-|strict|content-security|referrer|cache|server|permissions/i.test(k))))
// --- Idempotency ---
const idem = {}
const gI = await guestApi(); const k = randomUUID()
const itemId = LA.items[0].id
const r1 = await gI.post(`items/${itemId}/claims`, { data: { qty: 1, display_name: '冪等一' }, headers: { 'Idempotency-Key': k } }); const j1 = await r1.json()
const r2 = await gI.post(`items/${itemId}/claims`, { data: { qty: 1, display_name: '冪等一' }, headers: { 'Idempotency-Key': k } })
const r3 = await gI.post(`items/${itemId}/claims`, { data: { qty: 2, display_name: '冪等一' }, headers: { 'Idempotency-Key': k } })
idem.first = r1.status(); idem.replay = [r2.status(), r2.headers()['idempotency-replayed'], (await r2.json()).claim?.id === j1.claim.id]; idem.sameKeyDifferentBody = [r3.status(), (await r3.json()).code]
const r4 = await (await guestApi()).post(`items/${itemId}/claims`, { data: { qty: 1, display_name: '冪等一' }, headers: { 'Idempotency-Key': k } })
const j4 = await r4.json(); idem.otherAnonSameKeySameBody = [r4.status(), r4.headers()['idempotency-replayed'], j4.claim?.id === j1.claim.id, j4.claim?.claimer_name]
idem.claimsCreatedByKey = sql(`select count(*) from claims where item_id='${itemId}' and claimer_name='冪等一'`)
idem.guestTokenPlaintextInIdemTable = sql(`select count(*) from idempotency_keys where response_body::text like '%guest_token%'`)
idem.tokenAnywhereInTable = sql(`select count(*) from idempotency_keys i where position('${j1.guest_token}' in i.response_body::text) > 0 or position('${j1.guest_token}' in i.scope) > 0`)
idem.replayHasNoSetCookie = r2.headers()['set-cookie'] ?? null
L('idempotency', idem)
// --- 錯誤洩漏 ---
const leak = {}
for (const [n, f] of Object.entries({ 'nul title': () => A.api.post('wishlists', { data: { type: 'registry', title: 'a\u0000b' } }), 'bad uuid': () => A.api.get('wishlists/zzz'), 'bad json': () => raw.post(API + '/api/v1/auth/login', { data: '{', headers: { 'Content-Type': 'application/json' } }), 'wrong type': () => raw.post(API + '/api/v1/auth/login', { data: { email: 1, password: [] } }), 'ghost route': () => raw.get(API + '/api/v1/../../etc/passwd'), 'login unknown': () => raw.post(API + '/api/v1/auth/login', { data: { email: 'nobody@example.com', password: 'x' } }), 'login wrong pw': () => raw.post(API + '/api/v1/auth/login', { data: { email: A.email, password: 'wrong-pw-1' } }) })) {
  const r = await f(); const t = await r.text(); leak[n] = { status: r.status(), body: t.slice(0, 160), mentionsInternals: /sqlx|postgres|panicked|stack|src\/|\.rs|0x00|RUST_BACKTRACE|axum|serde/i.test(t) }
}
L('error-leak', leak)
// --- 帳號列舉 ---
const enumr = {}
const regExisting = await raw.post(API + '/api/v1/auth/register', { data: { email: A.email, password: PASSWORD, display_name: 'dup' }, headers: { Origin: WEB } }); enumr.registerExisting = [regExisting.status(), (await regExisting.text()).slice(0, 120)]
const regNew = await raw.post(API + '/api/v1/auth/register', { data: { email: `enum-${Date.now()}@example.com`, password: PASSWORD, display_name: 'new' }, headers: { Origin: WEB } }); enumr.registerNew = [regNew.status(), (await regNew.text()).slice(0, 120)]
const rs1 = await raw.post(API + '/api/v1/auth/password/reset/request', { data: { email: A.email }, headers: { Origin: WEB } }); const rs2 = await raw.post(API + '/api/v1/auth/password/reset/request', { data: { email: `nobody-${Date.now()}@example.com` }, headers: { Origin: WEB } }); enumr.resetKnownVsUnknown = [rs1.status(), rs2.status()]
L('enumeration', enumr)
// --- 限流 ---
const rl = {}
sql('delete from rate_limits')
const victim = await registerUser({ tag: 'rl' })
const codes = []; for (let i = 0; i < 40; i++) { const r = await raw.post(API + '/api/v1/auth/login', { data: { email: victim.email, password: 'bad-pass-' + i }, headers: { Origin: WEB } }); codes.push(r.status()); }
rl.loginFailures = codes.join(','); const okAfter = await raw.post(API + '/api/v1/auth/login', { data: { email: victim.email, password: PASSWORD }, headers: { Origin: WEB } }); rl.correctPasswordAfterLockout = okAfter.status(); rl.retryAfter = okAfter.headers()['retry-after'] ?? null
// 鎖定是否可被第三人用來鎖死受害者（by_email 5 次/5 分）
rl.lockoutIsPerEmail = '見上：任何人連續 5 次錯誤即鎖該 email 5 分鐘（含正確密碼）'
// 檢舉
const rc = []; for (let i = 0; i < 12; i++) { const r = await (await guestApi()).post(`public/wishlists/${LA.slug}/reports`, { data: { reason: 'other', detail: 'rl' + i } }); rc.push(r.status()) } rl.reportSameList = rc.join(',')
const LC = []; for (let i = 0; i < 12; i++) { const l = await createList(B, { title: 'rl' + i, items: [{ title: 'x', qty: 1 }] }); LC.push(l) }
const rc2 = []; for (const l of LC) { const r = await (await guestApi()).post(`public/wishlists/${l.slug}/reports`, { data: { reason: 'other' } }); rc2.push(r.status()) } rl.reportManyLists = rc2.join(',')
// 認領：同一 guest 15 次 / 同清單
const LD = await createList(B, { title: 'rl-claim', items: Array.from({ length: 20 }, (_, i) => ({ title: 'i' + i, qty: 3 })) })
const first = await guestClaim(LD.items[0].id, 1, '限流客'); const cs = [first.status]
for (let i = 1; i < 20; i++) { const r = await first.api.post(`items/${LD.items[i].id}/claims`, { data: { qty: 1 }, headers: { 'Idempotency-Key': randomUUID(), 'X-Guest-Token': first.token } }); cs.push(r.status()) } rl.claimsSameGuestSameList = cs.join(',')
// 新 guest 建立：按 IP 100/h（先前已重置）
let blockedAt = null; for (let i = 0; i < 110; i++) { const r = await (await guestApi()).post(`items/${LD.items[1].id}/claims`, { data: { qty: 1, display_name: 'n' + i }, headers: { 'Idempotency-Key': randomUUID() } }); if (r.status() === 429) { blockedAt = i; break } if (r.status() !== 201 && r.status() !== 409) { blockedAt = 'other ' + r.status(); break } } rl.newGuestsPerIpBlockedAfter = blockedAt
// OTP 請求
const oc = []; for (let i = 0; i < 4; i++) { const r = await raw.post(API + '/api/v1/auth/otp/request', { data: { email: victim.email }, headers: { Origin: WEB } }); oc.push(r.status()) } rl.otpRequest = oc.join(',')
L('ratelimit', rl)
console.log('done')
