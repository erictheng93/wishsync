// 創建者端 mock API：node mocks/creator-mock.mjs  （port 8080，無相依）
// OTP 一律 123456；email 含 "staff" 為 staff；圖片 PUT 收下即丟。
import http from 'node:http'
import { randomUUID } from 'node:crypto'

const PORT = process.env.PORT || 8080
const ORIGIN = process.env.WEB_ORIGIN || 'http://localhost:3000'
const uid = () => randomUUID()
const now = () => new Date().toISOString()
const sessions = new Map() // token -> user
const users = new Map()    // email -> user
const pending = new Map()  // email -> 待驗證註冊
const pws = new Map([['demo@example.com', 'password123']]) // email -> 密碼（mock 明文）
const lists = new Map()
const itemsBy = new Map()  // wishlist id -> items[]
const claims = []
const reports = [{ id: uid(), wishlist: null, item_id: null, reason: 'scam', detail: '要求私下轉帳', status: 'open', reporter: 'guest', handled_at: null, created_at: now() }]
const sse = new Set()
const flags = { read_only: false }

const send = (res, code, body, headers = {}) => {
  res.writeHead(code, { 'content-type': 'application/json', ...headers })
  res.end(body === undefined ? '' : JSON.stringify(body))
}
const problem = (res, status, code, detail, extra = {}) =>
  send(res, status, { type: 'about:blank', title: code, status, detail, code, ...extra }, { 'content-type': 'application/problem+json' })

const pub = (w) => ({ ...w, share_url: `${ORIGIN}/s/${w.slug}`, surprise_locked: !!(w.surprise_mode && w.event_date && Date.now() < unlockAt(w)) })
const unlockAt = (w) => Date.parse(w.event_date + 'T00:00:00+08:00')
const myItems = (w) => itemsBy.get(w.id) || []
const completion = (w) => {
  const it = myItems(w), f = it.filter((i) => i.qty_claimed >= i.qty_needed).length
  return { item_count: it.length, fulfilled_count: f, completion_pct: it.length ? Math.round((f / it.length) * 100) : 0 }
}
const maskItem = (i, w) => (pub(w).surprise_locked ? { ...i, qty_claimed: null } : i)

function seed() {
  const u = { id: uid(), display_name: 'demo', email: 'demo@example.com', avatar_url: null, is_staff: false, notification_prefs: { email_claims: true }, identities: [{ provider: 'email' }], orgs: [] }
  users.set(u.email, u)
  const mk = (title, extra = {}) => {
    const w = { id: uid(), type: 'registry', status: 'active', visibility: 'link', slug: Math.random().toString(36).slice(2, 12), title, description: null, cover_image_url: null, cover_image_status: 'none', event_date: null, show_claimer_names: false, surprise_mode: false, claim_ttl_hours: null, moderation_status: 'ok', moderation_reason: null, moderated_at: null, has_shipping_address: false, org_id: null, owner: u, created_at: now(), updated_at: now(), ...extra }
    lists.set(w.id, w)
    itemsBy.set(w.id, [])
    return w
  }
  const a = mk('寶寶迎接清單')
  const b = mk('驚喜生日清單', { surprise_mode: true, event_date: new Date(Date.now() + 3 * 864e5).toISOString().slice(0, 10) })
  mk('已下架範例', { moderation_status: 'hidden', moderation_reason: '疑似詐騙：要求私下轉帳' })
  for (const [w, t, n, c] of [[a, '玻璃奶瓶 240ml', 10, 6], [a, 'NB 尿布 2 包', 2, 2], [b, '藍牙耳機', 1, 1]]) {
    const it = { id: uid(), wishlist_id: w.id, title: t, description: null, brand: null, spec: null, image_url: null, image_status: 'none', product_url: null, unit_price_amount: 450, funding_mode: 'quantity', priority: 'medium', qty_needed: n, qty_claimed: c, qty_received: 0, target_points: null, pledged_points: null, funding_status: null, sort_order: itemsBy.get(w.id).length * 10 + 10, created_at: now(), updated_at: now() }
    itemsBy.get(w.id).push(it)
    claims.push({ id: uid(), item_id: it.id, qty: c, status: 'purchased', claimer_name: '阿明', note: null, created_at: now() })
  }
  reports[0].wishlist = { id: a.id, slug: a.slug, title: a.title }
}
seed()

function broadcast(w, event, data) {
  for (const c of sse) if (c.slug === w.slug) c.res.write(`id: ${Date.now()}\nevent: ${event}\ndata: ${JSON.stringify(data)}\n\n`)
}

const readBody = (req) => new Promise((r) => { let s = ''; req.on('data', (d) => (s += d)); req.on('end', () => { try { r(s ? JSON.parse(s) : {}) } catch { r({}) } }) })
const cookie = (req, k) => (req.headers.cookie || '').split(/;\s*/).map((x) => x.split('=')).find((x) => x[0] === k)?.[1]

http.createServer(async (req, res) => {
  const o = req.headers.origin
  const cors = { 'access-control-allow-origin': o || ORIGIN, 'access-control-allow-credentials': 'true', 'access-control-allow-methods': 'GET,POST,PATCH,PUT,DELETE,OPTIONS', 'access-control-allow-headers': 'Content-Type,Idempotency-Key,X-Guest-Token', vary: 'Origin' }
  for (const k in cors) res.setHeader(k, cors[k])
  if (req.method === 'OPTIONS') return send(res, 204)
  const url = new URL(req.url, 'http://x')
  const p = url.pathname.replace(/^\/api\/v1/, '')
  const m = req.method

  // 圖片直傳目標（假 R2）
  if (p.startsWith('/mock-upload/')) { req.resume(); return req.on('end', () => send(res, 200)) }

  // SSE
  let g
  if ((g = p.match(/^\/public\/wishlists\/([^/]+)\/events$/))) {
    const w = [...lists.values()].find((x) => x.slug === g[1])
    if (!w) return problem(res, 404, 'NOT_FOUND', '找不到清單')
    if (w.moderation_status === 'hidden') return problem(res, 410, 'WISHLIST_REMOVED', '此清單已被下架')
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache, no-transform' })
    res.write('retry: 5000\n: heartbeat\n\n')
    const c = { res, slug: w.slug }; sse.add(c)
    const hb = setInterval(() => res.write(': heartbeat\n\n'), 15000)
    return req.on('close', () => { sse.delete(c); clearInterval(hb) })
  }

  const body = ['POST', 'PATCH', 'PUT', 'DELETE'].includes(m) ? await readBody(req) : {}
  const token = cookie(req, 'ws_session')
  const me = sessions.get(token)
  const authed = () => (me ? true : (problem(res, 401, 'UNAUTHORIZED', '請先登入'), false))
  const staff = () => (authed() ? (me.is_staff ? true : (problem(res, 403, 'STAFF_ONLY', '這個頁面僅限營運人員'), false)) : false)
  if (flags.read_only && m !== 'GET' && !p.startsWith('/admin')) return problem(res, 503, 'READ_ONLY_MODE', '系統維護中，稍後再試', { 'retry-after': '60' })
  const ownList = (id) => { const w = lists.get(id); return w && w.owner.id === me.id ? w : null }
  const findItem = (id) => { for (const [wid, a] of itemsBy) { const i = a.find((x) => x.id === id); if (i) return { w: lists.get(wid), i, a } } }

  // ---- Auth ----
  // 密碼登入：demo@example.com / password123；email 含 "limit" 回 429；OTP 一律 123456
  const bad422 = (ptr, detail) => problem(res, 422, 'VALIDATION_FAILED', '請求內容有 1 個欄位不正確。', { errors: [{ pointer: ptr, code: 'FORMAT', detail }] })
  const okEmail = /^\S+@\S+\.\S+$/.test(String(body.email || ''))
  const startSession = (u, isNew, code = 200) => { const t = uid(); sessions.set(t, u); return send(res, code, { user: { id: u.id, display_name: u.display_name, email: u.email, avatar_url: null }, is_new_user: isNew }, { 'set-cookie': `ws_session=${t}; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax` }) }
  if (p === '/auth/login' && m === 'POST') {
    const e = String(body.email || '').trim().toLowerCase()
    if (e.includes('limit')) return problem(res, 429, 'RATE_LIMITED', '請求過於頻繁，請 300 秒後再試。', { 'retry-after': '300' })
    if (!pws.has(e) || pws.get(e) !== body.password) return problem(res, 401, 'INVALID_CREDENTIALS', 'Email 或密碼錯誤')
    return startSession(users.get(e), false)
  }
  if (p === '/auth/register' && m === 'POST') {
    if (!okEmail) return bad422('/email', 'Email 格式不正確')
    if (String(body.password || '').length < 8) return bad422('/password', '密碼長度須為 8–128 字元')
    pending.set(body.email.trim().toLowerCase(), { pw: body.password, name: String(body.display_name || '').trim() || 'user' })
    return send(res, 202, { expires_in: 600, resend_after: 20 })
  }
  if (p === '/auth/register/verify' && m === 'POST') {
    const e = String(body.email || '').trim().toLowerCase(), pd = pending.get(e)
    if (body.code !== '123456' || !pd) return problem(res, 400, 'OTP_INVALID', '驗證碼錯誤或已過期，請重新取得。')
    if (pws.has(e)) return problem(res, 409, 'EMAIL_EXISTS', '此 Email 已註冊，請直接登入或使用忘記密碼。')
    pending.delete(e)
    const u = users.get(e) || { id: uid(), display_name: pd.name, email: e, avatar_url: null, is_staff: e.includes('staff'), notification_prefs: { email_claims: true }, identities: [{ provider: 'email' }], orgs: [] }
    users.set(e, u); pws.set(e, pd.pw)
    return startSession(u, true)
  }
  if (p === '/auth/password/reset/request' && m === 'POST') {
    if (!okEmail) return bad422('/email', 'Email 格式不正確')
    return send(res, 202, { expires_in: 600, resend_after: 20 })
  }
  if (p === '/auth/password/reset/confirm' && m === 'POST') {
    if (String(body.new_password || '').length < 8) return bad422('/new_password', '密碼長度須為 8–128 字元')
    if (body.code !== '123456') return problem(res, 400, 'OTP_INVALID', '驗證碼錯誤或已過期，請重新取得。')
    pws.set(String(body.email).trim().toLowerCase(), body.new_password)
    return send(res, 204)
  }
  if (p === '/auth/otp/request' && m === 'POST') {
    if (!/^\S+@\S+\.\S+$/.test(body.email || '')) return problem(res, 422, 'VALIDATION_FAILED', '請求內容有 1 個欄位不正確。', { errors: [{ pointer: '/email', code: 'FORMAT', detail: 'Email 格式不正確' }] })
    return send(res, 202, { expires_in: 600, resend_after: 20 })
  }
  if (p === '/auth/otp/verify' && m === 'POST') {
    if (body.code !== '123456') return problem(res, 401, 'OTP_INVALID', '驗證碼不正確')
    let u = users.get(body.email), isNew = false
    if (!u) { isNew = true; u = { id: uid(), display_name: body.email.split('@')[0], email: body.email, avatar_url: null, is_staff: body.email.includes('staff'), notification_prefs: { email_claims: true }, identities: [{ provider: 'email' }], orgs: [] }; users.set(body.email, u) }
    const t = uid(); sessions.set(t, u)
    return send(res, 200, { user: { id: u.id, display_name: u.display_name, email: u.email, avatar_url: null }, is_new_user: isNew }, { 'set-cookie': `ws_session=${t}; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax` })
  }
  if (g = p.match(/^\/auth\/oauth\/(line|google)\/start$/)) { res.writeHead(302, { location: `${ORIGIN}/login?error=oauth_failed` }); return res.end() }
  if (p === '/auth/logout' && m === 'POST') { sessions.delete(token); return send(res, 204, undefined, { 'set-cookie': 'ws_session=; Max-Age=0; Path=/' }) }

  // ---- Me ----
  if (p === '/me') {
    if (!authed()) return
    if (m === 'GET') return send(res, 200, me)
    if (m === 'PATCH') {
      if (body.display_name) me.display_name = body.display_name
      if (body.notification_prefs) { if (Object.keys(body.notification_prefs).some((k) => k !== 'email_claims')) return problem(res, 422, 'VALIDATION_FAILED', '未知的偏好'); Object.assign(me.notification_prefs, body.notification_prefs) }
      return send(res, 200, me)
    }
    if (m === 'DELETE') {
      if (body.confirm !== 'DELETE') return problem(res, 422, 'VALIDATION_FAILED', '請帶 confirm: "DELETE"')
      sessions.delete(token); users.delete(me.email)
      return send(res, 200, { deleted: true, anonymized_at: now() }, { 'set-cookie': 'ws_session=; Max-Age=0; Path=/' })
    }
  }
  if (p === '/me/export') { if (!authed()) return; return send(res, 200, { exported_at: now(), user: me, wishlists: [...lists.values()].filter((w) => w.owner.id === me.id).map((w) => ({ id: w.id, title: w.title, items: myItems(w) })), claims: [] }) }

  // ---- Wishlists ----
  if (p === '/wishlists' && m === 'POST') {
    if (!authed()) return
    if (!body.title?.trim()) return problem(res, 422, 'VALIDATION_FAILED', '請求內容有 1 個欄位不正確。', { errors: [{ pointer: '/title', code: 'REQUIRED', detail: '請輸入清單名稱' }] })
    if (body.surprise_mode && (!body.event_date || Date.parse(body.event_date) <= Date.now())) return problem(res, 422, 'VALIDATION_FAILED', '活動日需為未來日期', { errors: [{ pointer: '/event_date', code: 'RANGE', detail: '驚喜模式需設定未來的活動日' }] })
    const w = { id: uid(), type: body.type, status: 'draft', visibility: 'link', slug: Math.random().toString(36).slice(2, 12), title: body.title, description: body.description ?? null, cover_image_url: body.cover_image_key ? `${ORIGIN}/favicon.ico` : null, cover_image_status: 'none', event_date: body.event_date ?? null, show_claimer_names: !!body.show_claimer_names, surprise_mode: !!body.surprise_mode, claim_ttl_hours: null, moderation_status: 'ok', moderation_reason: null, moderated_at: null, has_shipping_address: false, org_id: null, owner: me, created_at: now(), updated_at: now() }
    lists.set(w.id, w); itemsBy.set(w.id, [])
    return send(res, 201, pub(w))
  }
  if (p === '/wishlists' && m === 'GET') {
    if (!authed()) return
    const data = [...lists.values()].filter((w) => w.owner.id === me.id && w.status !== 'archived').reverse().map((w) => ({ id: w.id, type: w.type, status: w.status, slug: w.slug, title: w.title, event_date: w.event_date, cover_image_url: w.cover_image_url, surprise_locked: pub(w).surprise_locked, moderation_status: w.moderation_status, moderation_reason: w.moderation_reason, completion: completion(w), updated_at: w.updated_at }))
    return send(res, 200, { data, next_cursor: null })
  }
  if ((g = p.match(/^\/wishlists\/([^/]+)$/))) {
    if (!authed()) return
    const w = ownList(g[1]); if (!w) return problem(res, 404, 'NOT_FOUND', '找不到清單')
    if (m === 'GET') return send(res, 200, { wishlist: pub(w), items: myItems(w).map((i) => maskItem(i, w)) })
    if (m === 'PATCH') {
      if (body.status === 'active' && w.status === 'draft') {
        const errors = []
        if (!w.title.trim()) errors.push({ pointer: '/title', code: 'REQUIRED', detail: '清單名稱不可為空' })
        if (!myItems(w).length) errors.push({ pointer: '/items', code: 'REQUIRED', detail: '至少需要 1 個品項' })
        if (errors.length) return problem(res, 409, 'WISHLIST_NOT_PUBLISHABLE', '清單尚不符合發佈條件。', { errors })
      }
      for (const k of ['title', 'description', 'event_date', 'visibility', 'status', 'show_claimer_names', 'surprise_mode', 'cover_image_key']) if (k in body) w[k] = body[k]
      w.updated_at = now(); broadcast(w, 'wishlist.updated', { status: w.status, title: w.title, completion: completion(w), updated_at: w.updated_at })
      return send(res, 200, pub(w))
    }
    if (m === 'DELETE') { w.status = 'archived'; return send(res, 204) }
  }
  if ((g = p.match(/^\/wishlists\/([^/]+)\/dashboard$/)) && m === 'GET') {
    if (!authed()) return
    const w = ownList(g[1]); if (!w) return problem(res, 404, 'NOT_FOUND', '找不到清單')
    const locked = pub(w).surprise_locked, it = myItems(w)
    return send(res, 200, {
      wishlist_id: w.id, surprise_locked: locked, unlock_at: w.event_date ? new Date(unlockAt(w)).toISOString() : null,
      totals: { ...completion(w), qty_needed: it.reduce((s, i) => s + i.qty_needed, 0), qty_claimed: it.reduce((s, i) => s + i.qty_claimed, 0), target_points: 0, pledged_points: 0, funded_item_count: 0 },
      items: it.map((i) => ({ item_id: i.id, title: i.title, funding_mode: 'quantity', qty_needed: i.qty_needed, qty_claimed: locked ? null : i.qty_claimed, target_points: null, pledged_points: null, funding_status: null })),
      orders_summary: { funded_count: 0, ordered_count: 0, shipped_count: 0, delivered_count: 0 },
      claims: locked ? null : claims.filter((c) => it.some((i) => i.id === c.item_id)), contributions: locked ? null : [],
    })
  }
  if ((g = p.match(/^\/wishlists\/([^/]+)\/items$/)) && m === 'POST') {
    if (!authed()) return
    const w = ownList(g[1]); if (!w) return problem(res, 404, 'NOT_FOUND', '找不到清單')
    if (['closed', 'archived'].includes(w.status)) return problem(res, 409, 'WISHLIST_CLOSED', '清單已結束')
    if (!body.title) return problem(res, 422, 'VALIDATION_FAILED', '請輸入名稱', { errors: [{ pointer: '/title', code: 'REQUIRED', detail: '請輸入品項名稱' }] })
    const a = itemsBy.get(w.id)
    const i = { id: uid(), wishlist_id: w.id, description: null, brand: null, spec: null, image_url: null, image_status: 'none', product_url: null, unit_price_amount: null, funding_mode: 'quantity', priority: 'medium', qty_claimed: 0, qty_received: 0, target_points: null, pledged_points: null, funding_status: null, sort_order: a.length * 10 + 10, created_at: now(), updated_at: now(), ...pickItem(body) }
    if (body.image_key) setImage(i)
    a.push(i); broadcast(w, 'item.updated', { item_id: i.id, qty_needed: i.qty_needed, qty_claimed: 0 })
    return send(res, 201, i)
  }
  if ((g = p.match(/^\/wishlists\/([^/]+)\/items\/reorder$/)) && m === 'POST') {
    if (!authed()) return
    const w = ownList(g[1]); if (!w) return problem(res, 404, 'NOT_FOUND', '找不到清單')
    const a = itemsBy.get(w.id), ids = body.item_ids || []
    if (ids.length !== a.length || !ids.every((x) => a.some((i) => i.id === x))) return problem(res, 422, 'VALIDATION_FAILED', 'id 集合不一致')
    itemsBy.set(w.id, ids.map((x, k) => Object.assign(a.find((i) => i.id === x), { sort_order: (k + 1) * 10 })))
    return send(res, 204)
  }
  if ((g = p.match(/^\/items\/([^/]+)$/))) {
    if (!authed()) return
    const f = findItem(g[1]); if (!f || f.w.owner.id !== me.id) return problem(res, 404, 'NOT_FOUND', '找不到品項')
    if (m === 'PATCH') {
      if (body.expected_updated_at && body.expected_updated_at !== f.i.updated_at) return problem(res, 409, 'STALE_VERSION', '這份資料已在其他地方被修改，請重新載入後再試。')
      if (body.qty_needed !== undefined && body.qty_needed < f.i.qty_claimed) return problem(res, 409, 'QTY_BELOW_CLAIMED', `數量不可低於已認領的 ${f.i.qty_claimed} 件`)
      Object.assign(f.i, pickItem(body), { updated_at: now() }); if (body.image_key) setImage(f.i)
      broadcast(f.w, 'item.updated', { item_id: f.i.id, qty_needed: f.i.qty_needed, qty_claimed: f.i.qty_claimed })
      return send(res, 200, f.i)
    }
    if (m === 'DELETE') {
      if (f.i.qty_claimed > 0 && url.searchParams.get('force') !== 'true') return problem(res, 409, 'ITEM_HAS_CLAIMS', '此品項已有認領，需帶 force=true')
      f.a.splice(f.a.indexOf(f.i), 1); broadcast(f.w, 'item.updated', { item_id: f.i.id, deleted: true })
      return send(res, 204)
    }
  }
  if ((g = p.match(/^\/claims\/([^/]+)$/)) && m === 'PATCH') {
    if (!authed()) return
    const c = claims.find((x) => x.id === g[1]); if (!c) return problem(res, 404, 'NOT_FOUND', '找不到認領')
    c.status = body.status; return send(res, 200, { claim: c })
  }
  if (p === '/uploads/presign' && m === 'POST') {
    if (!authed()) return
    if (!['image/jpeg', 'image/png', 'image/webp'].includes(body.content_type)) return problem(res, 422, 'VALIDATION_FAILED', '格式不符', { errors: [{ pointer: '/content_type', code: 'ENUM', detail: '只允許 JPG、PNG、WebP' }] })
    if (body.content_length > 5 * 1024 * 1024) return problem(res, 422, 'VALIDATION_FAILED', '檔案過大', { errors: [{ pointer: '/content_length', code: 'RANGE', detail: '圖片需在 5 MB 以內' }] })
    const key = `${body.purpose}s/${uid()}.webp`
    return send(res, 200, { upload_url: `http://localhost:${PORT}/mock-upload/${key}`, method: 'PUT', headers: { 'Content-Type': body.content_type, 'Content-Length': String(body.content_length) }, object_key: key, public_url: `${ORIGIN}/favicon.ico`, processing: true, expires_at: now() })
  }

  // ---- Admin ----
  if (p.startsWith('/admin/')) {
    if (!staff()) return
    if (p === '/admin/reports' && m === 'GET') { const s = url.searchParams.get('status') || 'open'; return send(res, 200, { data: reports.filter((r) => r.status === s), next_cursor: null }) }
    if ((g = p.match(/^\/admin\/reports\/([^/]+)$/)) && m === 'PATCH') {
      const r = reports.find((x) => x.id === g[1]); if (!r) return problem(res, 404, 'NOT_FOUND', '找不到檢舉')
      if (r.status !== 'open') return problem(res, 409, 'INVALID_STATE_TRANSITION', '此檢舉已處理')
      r.status = body.status; r.handled_at = now(); return send(res, 200, r)
    }
    if ((g = p.match(/^\/admin\/wishlists\/([^/]+)\/moderation$/)) && m === 'PATCH') {
      const w = lists.get(g[1]); if (!w) return problem(res, 404, 'NOT_FOUND', '找不到清單')
      if (body.moderation_status === 'hidden' && !body.reason) return problem(res, 422, 'VALIDATION_FAILED', '下架需填原因', { errors: [{ pointer: '/reason', code: 'REQUIRED', detail: '下架需填原因' }] })
      w.moderation_status = body.moderation_status; w.moderation_reason = body.moderation_status === 'hidden' ? body.reason : null; w.moderated_at = now()
      if (w.moderation_status === 'hidden') reports.forEach((r) => r.wishlist?.id === w.id && r.status === 'open' && Object.assign(r, { status: 'actioned', handled_at: now() }))
      return send(res, 200, { id: w.id, slug: w.slug, moderation_status: w.moderation_status, moderation_reason: w.moderation_reason, moderated_at: w.moderated_at })
    }
    if (p === '/admin/wishlists' && m === 'GET') {
      const q = (url.searchParams.get('q') || '').toLowerCase()
      const data = [...lists.values()].filter((w) => !q || w.slug.toLowerCase() === q || w.owner.email.startsWith(q)).map((w) => ({ id: w.id, slug: w.slug, title: w.title, status: w.status, type: w.type, owner: { id: w.owner.id, display_name: w.owner.display_name, email: w.owner.email }, moderation_status: w.moderation_status, open_report_count: reports.filter((r) => r.wishlist?.id === w.id && r.status === 'open').length, created_at: w.created_at }))
      return send(res, 200, { data, next_cursor: null })
    }
    if (p === '/admin/users' && m === 'GET') {
      const q = (url.searchParams.get('q') || '').toLowerCase()
      return send(res, 200, { data: [...users.values()].filter((u) => !q || u.email.startsWith(q) || u.id === q).map((u) => ({ id: u.id, display_name: u.display_name, email: u.email, is_staff: u.is_staff, blocked: false, deleted: false, wishlist_count: [...lists.values()].filter((w) => w.owner.id === u.id).length, created_at: now() })), next_cursor: null })
    }
    if (p === '/admin/system-flags/read_only' && m === 'PUT') { flags.read_only = !!body.value; return send(res, 200, { key: 'read_only', value: flags.read_only, updated_at: now() }) }
  }

  // ---- 測試輔助：模擬他人認領，觸發 SSE ----
  if (p === '/mock/claim' && m === 'POST') {
    const f = findItem(body.item_id); if (!f) return problem(res, 404, 'NOT_FOUND', 'x')
    f.i.qty_claimed = Math.min(f.i.qty_needed, f.i.qty_claimed + 1)
    claims.push({ id: uid(), item_id: f.i.id, qty: 1, status: 'reserved', claimer_name: '小美', note: null, created_at: now() })
    broadcast(f.w, 'item.updated', { item_id: f.i.id, qty_claimed: f.i.qty_claimed })
    return send(res, 200, f.i)
  }
  problem(res, 404, 'NOT_FOUND', '找不到路徑')
}).listen(PORT, () => console.log(`creator mock on :${PORT}（OTP 123456；含 staff 的 email 為營運人員；demo@example.com 有種子資料）`))

function pickItem(b) {
  const o = {}
  for (const k of ['title', 'brand', 'spec', 'product_url', 'unit_price_amount', 'priority', 'qty_needed']) if (k in b) o[k] = b[k]
  return o
}
function setImage(i) { i.image_status = 'pending'; setTimeout(() => { i.image_status = 'ready'; i.image_url = `${ORIGIN}/favicon.ico` }, 3000) }
