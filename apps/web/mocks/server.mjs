// 訪客端契約 mock（docs 04 §5）。node mocks/server.mjs  → http://localhost:8080
// slug: demo=正常 / closed=已結束 / gone=410 / boom=500 / sseoff=清單正常但 events 回 500（測輪詢備援）/ 其他=404。恢復權杖 "good" 有效。
import http from 'node:http'
import { randomBytes } from 'node:crypto'

const items = [
  { id: 'i1', title: '玻璃奶瓶 240ml', brand: 'Pigeon', spec: '寬口徑 / 3 入裝', image_url: null, product_url: 'https://example.com', unit_price_amount: 450, priority: 'high', qty_needed: 10, qty_claimed: 6 },
  { id: 'i2', title: 'NB 尿布 2 包', brand: null, spec: null, image_url: null, product_url: null, unit_price_amount: 300, priority: 'medium', qty_needed: 2, qty_claimed: 1 },
  { id: 'i3', title: '奶粉 1 號 800g', brand: null, spec: null, image_url: null, product_url: null, unit_price_amount: 800, priority: 'low', qty_needed: 2, qty_claimed: 2 },
  // 點數眾籌品項（P2-A）：沒有後端也能預覽卡片；/me 回 401，所以點「用點數贊助」會導到登入頁
  { id: 'i4', title: 'Combi 嬰兒推車', brand: 'Combi', spec: null, image_url: null, product_url: null, unit_price_amount: null, priority: 'high', qty_needed: 1, qty_claimed: 0,
    funding_mode: 'crowdfund', target_points: 9800, pledged_points: 6300, remaining_points: 3500, funding_status: 'open', display_status: 'open', funding_deadline: '2026-12-19T15:59:00Z',
    contributors: [{ display_name: '阿明', points: 3000 }, { display_name: '匿名朋友', points: 1300 }, { display_name: '小華', points: 2000 }] },
]
const view = i => { const r = i.qty_needed - i.qty_claimed; return { funding_mode: 'quantity', ...i, qty_remaining: r, is_fully_claimed: r <= 0, progress_percent: i.target_points ? Math.floor(i.pledged_points / i.target_points * 100) : Math.round(i.qty_claimed / i.qty_needed * 100) } }
const guests = new Map(), claims = new Map(), idem = new Map(), sse = new Set()
let n = 0

const send = (res, st, body, type = 'application/json') => { res.writeHead(st, { 'content-type': type }); res.end(body === undefined ? '' : JSON.stringify(body)) }
const err = (res, st, code, detail, extra = {}) => send(res, st, { type: 'about:blank', title: code, status: st, code, detail, ...extra }, 'application/problem+json')
const claimOut = c => ({ id: c.id, item_id: c.item_id, qty: c.qty, status: c.status, claimer_name: c.name, note: c.note, expires_at: null, created_at: c.at, updated_at: c.at, purchased_at: null, delivered_at: null, cancelled_at: null })
const broadcast = i => { const v = view(i); for (const r of sse) r.write(`event: item.updated\ndata: ${JSON.stringify({ item_id: i.id, qty_needed: v.qty_needed, qty_claimed: v.qty_claimed, qty_remaining: v.qty_remaining, is_fully_claimed: v.is_fully_claimed })}\n\n`) }

http.createServer(async (req, res) => {
  const origin = req.headers.origin
  if (origin) Object.entries({ 'access-control-allow-origin': origin, 'access-control-allow-credentials': 'true', 'access-control-allow-headers': 'content-type,idempotency-key,x-guest-token', 'access-control-allow-methods': 'GET,POST,PATCH,DELETE,OPTIONS', vary: 'origin' }).forEach(([k, v]) => res.setHeader(k, v))
  if (req.method === 'OPTIONS') return send(res, 204)
  const u = new URL(req.url, 'http://x'), p = u.pathname.replace('/api/v1', '')
  let body = {}
  if (req.method !== 'GET') { let s = ''; for await (const c of req) s += c; try { body = JSON.parse(s || '{}') } catch {} }
  const tok = req.headers['x-guest-token'], guest = tok && guests.get(tok)
  let m
  console.log(req.method, p)

  if ((m = p.match(/^\/public\/wishlists\/([^/]+)$/))) {
    if (m[1] === 'boom') return err(res, 500, 'INTERNAL', '內部錯誤')
    if (m[1] === 'gone') return err(res, 410, 'WISHLIST_REMOVED', '已下架')
    if (!['demo', 'closed', 'sseoff'].includes(m[1])) return err(res, 404, 'NOT_FOUND', '找不到')
    const list = items.map(view), done = list.filter(i => i.is_fully_claimed).length
    return send(res, 200, { slug: m[1], type: 'registry', status: m[1] === 'closed' ? 'closed' : 'active', title: '小愛的待產清單', description: '預產期 12 月，謝謝大家的心意', cover_image_url: null, event_date: '2026-12-20', id: 'w1', owner: { id: 'u1', display_name: '小愛' }, surprise_mode: false, claimers_visible: false, completion: { item_count: list.length, fulfilled_count: done, completion_pct: Math.round(items.reduce((a, i) => a + i.qty_claimed, 0) / items.reduce((a, i) => a + i.qty_needed, 0) * 100) }, items: list, updated_at: new Date().toISOString() })
  }
  if (p.match(/^\/public\/wishlists\/[^/]+\/events$/)) {
    if (p.includes('/sseoff/')) return err(res, 500, 'INTERNAL', '內部錯誤')
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' }); res.write('retry: 5000\n: heartbeat\n\n')
    sse.add(res); req.on('close', () => sse.delete(res)); return
  }
  if (p.match(/^\/public\/wishlists\/[^/]+\/reports$/)) {
    if (!body.turnstile_token) return err(res, 422, 'VALIDATION_FAILED', '驗證失敗', { errors: [{ pointer: '/turnstile_token', code: 'INVALID', detail: '驗證失敗' }] })
    return send(res, 201, { id: 'r' + ++n, status: 'open' })
  }
  if ((m = p.match(/^\/items\/([^/]+)\/claims$/)) && req.method === 'POST') {
    const key = req.headers['idempotency-key']; if (!key) return err(res, 400, 'IDEMPOTENCY_KEY_REQUIRED', '缺少 Idempotency-Key')
    if (idem.has(key)) return send(res, 201, idem.get(key))
    const it = items.find(i => i.id === m[1]); if (!it) return err(res, 404, 'NOT_FOUND', '找不到品項')
    if (!guest && !body.display_name) return err(res, 422, 'VALIDATION_FAILED', '請求內容有 1 個欄位不正確。', { errors: [{ pointer: '/display_name', code: 'REQUIRED', detail: '首次認領需填寫暱稱' }] })
    if (!(body.qty >= 1 && body.qty <= 99)) return err(res, 422, 'VALIDATION_FAILED', 'qty 不正確', { errors: [{ pointer: '/qty', code: 'RANGE', detail: 'qty 必須介於 1 與 99' }] })
    const rem = it.qty_needed - it.qty_claimed
    if (body.qty > rem) return err(res, 409, 'ITEM_FULLY_CLAIMED', `此品項剩餘 ${rem} 件，無法再認領 ${body.qty} 件。`, { remaining: rem })
    let gt, g = guest
    if (!g) { gt = randomBytes(32).toString('base64url'); g = { name: body.display_name, contact: body.contact }; guests.set(gt, g); g.token = gt }
    if ([...claims.values()].some(c => c.guest === g && c.item_id === it.id && c.status !== 'cancelled')) return err(res, 409, 'CLAIM_ALREADY_EXISTS', '已認領過', { claim_id: [...claims.values()].find(c => c.guest === g && c.item_id === it.id).id })
    it.qty_claimed += body.qty
    const c = { id: 'c' + ++n, item_id: it.id, qty: body.qty, status: 'reserved', name: g.name, note: body.note ?? null, at: new Date().toISOString(), guest: g }
    claims.set(c.id, c); broadcast(it)
    const out = { claim: claimOut(c), item: { id: it.id, ...pick(it) }, ...(gt ? { guest_token: gt } : {}) }
    idem.set(key, out); return send(res, 201, out)
  }
  if ((m = p.match(/^\/claims\/([^/]+)$/))) {
    const c = claims.get(m[1]); if (!guest) return err(res, 401, 'UNAUTHORIZED', '請先認領')
    if (!c || c.guest !== guest) return err(res, 404, 'NOT_FOUND', '找不到')
    const it = items.find(i => i.id === c.item_id)
    if (req.method === 'DELETE') { if (c.status !== 'cancelled') { it.qty_claimed -= c.qty; c.status = 'cancelled'; broadcast(it) } return send(res, 204) }
    if (body.qty && body.qty !== c.qty) { const d = body.qty - c.qty; if (d > it.qty_needed - it.qty_claimed) return err(res, 409, 'ITEM_FULLY_CLAIMED', '數量不足', { remaining: it.qty_needed - it.qty_claimed }); it.qty_claimed += d; c.qty = body.qty }
    if ('note' in body) c.note = body.note
    if (body.status === 'cancelled') { it.qty_claimed -= c.qty; c.status = 'cancelled' } else if (body.status) c.status = body.status
    broadcast(it); return send(res, 200, { claim: claimOut(c), item: { id: it.id, ...pick(it) } })
  }
  if (p === '/guest/me') {
    if (!guest) return err(res, 401, 'UNAUTHORIZED', '無有效 token')
    if (req.method === 'GET') return send(res, 200, { guest: { display_name: guest.name, contact: guest.contact ?? null }, claims: [...claims.values()].filter(c => c.guest === guest).reverse().map(c => { const it = items.find(i => i.id === c.item_id); return { claim: claimOut(c), item: { id: it.id, title: it.title, image_url: null }, wishlist: { slug: 'demo', title: '小愛的待產清單', event_date: '2026-12-20', status: 'active' } } }), next_cursor: null })
    if (req.method === 'PATCH') { guest.name = body.display_name ?? guest.name; return send(res, 200, { guest: { display_name: guest.name, contact: guest.contact ?? null } }) }
    if (req.method === 'DELETE') { guests.delete(tok); return send(res, 204) }
  }
  if (p === '/guest/recover') {
    if (body.token !== 'good') return err(res, 404, 'NOT_FOUND', '無效')
    const t = randomBytes(32).toString('base64url'); guests.set(t, { name: '阿明(恢復)', token: t }); return send(res, 200, { guest: { display_name: '阿明(恢復)' }, guest_token: t })
  }
  if (p === '/me') return err(res, 401, 'UNAUTHORIZED', '未登入')
  err(res, 404, 'NOT_FOUND', 'no route ' + p)
}).listen(8080, () => console.log('mock on :8080'))
const pick = i => ({ qty_needed: i.qty_needed, qty_claimed: i.qty_claimed, qty_remaining: i.qty_needed - i.qty_claimed })
