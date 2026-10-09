// QA 探索共用工具：獨立資源 wishsync_qa / API :8082 / Web :3014
import { request as pwRequest, chromium } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { randomBytes, randomUUID } from 'node:crypto'
import { mkdirSync, writeFileSync, appendFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const HERE = dirname(fileURLToPath(import.meta.url))
export const ROOT = resolve(HERE, '../../..')
export const OUT = resolve(HERE, 'out')
mkdirSync(OUT, { recursive: true })
export const API = 'http://localhost:8082'
export const WEB = 'http://localhost:3014'
export const MAILPIT = 'http://localhost:8025'
export const PASSWORD = 'qa-Passw0rd!'
export const uid = () => randomBytes(4).toString('hex')
export { chromium, pwRequest, randomUUID }

export function sql(stmt, db = 'wishsync_qa') {
  return execFileSync('docker', ['compose', 'exec', '-T', 'db', 'psql', '-U', 'wishsync', '-d', db, '-v', 'ON_ERROR_STOP=1', '-tA', '-c', stmt], { cwd: ROOT, encoding: 'utf8' }).trim()
}
export const q = s => String(s).replace(/'/g, "''")

export function log(file, obj) { appendFileSync(resolve(OUT, file), JSON.stringify(obj) + '\n') }
export const sleep = ms => new Promise(r => setTimeout(r, ms))

export async function waitForMail(to, { subject, timeout = 20000 } = {}) {
  const api = await pwRequest.newContext()
  const t0 = Date.now()
  try {
    while (Date.now() - t0 < timeout) {
      const r = await api.get(`${MAILPIT}/api/v1/search`, { params: { query: `to:"${to}"` } })
      const list = (await r.json()).messages ?? []
      const f = list.find(m => !subject || subject.test(m.Subject))
      if (f) { const m = await (await api.get(`${MAILPIT}/api/v1/message/${f.ID}`)).json(); return { id: f.ID, subject: f.Subject, text: m.Text, html: m.HTML } }
      await sleep(400)
    }
    throw new Error('no mail for ' + to)
  } finally { await api.dispose() }
}

export const apiCtx = () => pwRequest.newContext({ baseURL: API + '/api/v1/', extraHTTPHeaders: { Origin: WEB } })

export async function registerUser({ name, tag = 'u', staff = false } = {}) {
  const email = `qa-${tag}-${uid()}@example.com`, display = name ?? `創建者${uid().slice(0, 4)}`
  const api = await apiCtx()
  let r = await api.post('auth/register', { data: { email, password: PASSWORD, display_name: display } })
  if (!r.ok()) throw new Error('register ' + r.status() + await r.text())
  const m = await waitForMail(email)
  const code = m.text.match(/(\d{6})/)[1]
  r = await api.post('auth/register/verify', { data: { email, code } })
  if (!r.ok()) throw new Error('verify ' + r.status() + await r.text())
  if (staff) sql(`UPDATE users SET is_staff = true WHERE email = '${q(email)}'`)
  return { email, password: PASSWORD, name: display, api }
}

export async function createList(user, o = {}) {
  const title = o.title ?? `QA 清單 ${uid()}`
  const body = { type: 'registry', title, visibility: 'link', surprise_mode: !!o.surprise, show_claimer_names: !!o.showNames, ...(o.extra ?? {}) }
  if (o.surprise) body.event_date = new Date(Date.now() + 30 * 864e5 + 8 * 36e5).toISOString().slice(0, 10)
  let r = await user.api.post('wishlists', { data: body })
  if (!r.ok()) throw new Error('create ' + r.status() + await r.text())
  const w = await r.json()
  const items = []
  for (const it of o.items ?? [{ title: '奶瓶', qty: 1 }]) {
    r = await user.api.post(`wishlists/${w.id}/items`, { data: { title: it.title, qty_needed: it.qty ?? 1, priority: 'medium', funding_mode: 'quantity', ...(it.extra ?? {}) } })
    if (!r.ok()) throw new Error('item ' + r.status() + await r.text())
    items.push({ id: (await r.json()).id, title: it.title })
  }
  if (o.publish !== false) {
    const cur = await (await user.api.get(`wishlists/${w.id}`)).json()
    r = await user.api.patch(`wishlists/${w.id}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } })
    if (!r.ok()) throw new Error('publish ' + r.status() + await r.text())
  }
  const slug = (await (await user.api.get(`wishlists/${w.id}`)).json()).wishlist.slug
  return { id: w.id, slug, title, items }
}

export async function guestApi() { return pwRequest.newContext({ baseURL: API + '/api/v1/', extraHTTPHeaders: { Origin: WEB } }) }
export async function guestClaim(itemId, qty = 1, name = `訪客${uid().slice(0, 4)}`, extra = {}) {
  const api = await guestApi()
  const r = await api.post(`items/${itemId}/claims`, { data: { qty, display_name: name, ...extra }, headers: { 'Idempotency-Key': randomUUID() } })
  const j = await r.json().catch(() => ({}))
  return { status: r.status(), body: j, token: j.guest_token, api, claimId: j.claim?.id }
}

export async function sessionCookies(user) { return (await user.api.storageState()).cookies }

export const VIEWPORTS = [
  { name: '320', width: 320, height: 568 },
  { name: '390', width: 390, height: 844 },
  { name: '768', width: 768, height: 1024 },
  { name: '1440', width: 1440, height: 900 },
]

export async function newCtx(browser, { viewport, scheme = 'light', cookies, ...rest } = {}) {
  const ctx = await browser.newContext({ baseURL: WEB, locale: 'zh-TW', timezoneId: 'Asia/Taipei', colorScheme: scheme, viewport: viewport ?? { width: 390, height: 844 }, ...rest })
  if (cookies) await ctx.addCookies(cookies)
  return ctx
}

/** 蒐集 console / 請求問題 */
export function watch(page, tag, sink) {
  const push = (kind, text, extra = {}) => sink.push({ tag, kind, text: String(text).slice(0, 400), ...extra })
  page.on('console', m => { if (['error', 'warning'].includes(m.type())) push('console.' + m.type(), m.text(), { loc: m.location()?.url }) })
  page.on('pageerror', e => push('pageerror', e.message))
  page.on('requestfailed', r => push('requestfailed', `${r.method()} ${r.url()} ${r.failure()?.errorText}`))
  page.on('response', r => { if (r.status() >= 400) push('http' + r.status(), `${r.request().method()} ${r.url()}`) })
}

/** 頁內稽核：溢位、截斷、觸控目標、alt、label、對比 */
export const AUDIT_FN = () => {
  const out = { overflowX: false, truncated: [], smallTargets: [], noAlt: [], noLabel: [], lowContrast: [], noName: [] }
  const de = document.documentElement
  out.overflowX = de.scrollWidth > window.innerWidth
  if (out.overflowX) {
    out.overflowCulprits = [...document.querySelectorAll('body *')].filter(e => { const r = e.getBoundingClientRect(); return r.right > window.innerWidth + 1 && r.width > 0 }).slice(0, 5).map(e => e.tagName.toLowerCase() + '.' + (e.className || '').toString().slice(0, 40) + ' r=' + Math.round(e.getBoundingClientRect().right))
  }
  const sel = (e) => { let s = e.tagName.toLowerCase(); if (e.id) s += '#' + e.id; if (e.className && typeof e.className === 'string') s += '.' + e.className.trim().split(/\s+/).slice(0, 2).join('.'); return s + (e.textContent ? `「${e.textContent.trim().slice(0, 20)}」` : '') }
  const visible = e => { const r = e.getBoundingClientRect(); const cs = getComputedStyle(e); return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && cs.opacity !== '0' }
  for (const e of document.querySelectorAll('body *')) {
    if (!visible(e)) continue
    const cs = getComputedStyle(e)
    if ((cs.overflow === 'hidden' || cs.overflowX === 'hidden' || cs.textOverflow === 'ellipsis') && e.scrollWidth > e.clientWidth + 1 && e.clientWidth > 0 && e.children.length < 3) out.truncated.push(sel(e) + ` sw=${e.scrollWidth} cw=${e.clientWidth}`)
  }
  for (const e of document.querySelectorAll('a[href], button, input:not([type=hidden]), select, textarea, [role=button], summary')) {
    if (!visible(e)) continue
    const r = e.getBoundingClientRect()
    if (e.tagName === 'A' && getComputedStyle(e).display === 'inline' && e.closest('p, li, div') && e.parentElement.textContent.trim().length > e.textContent.trim().length + 3) continue // 行內連結例外(WCAG 2.5.8)
    if (r.width < 44 || r.height < 44) out.smallTargets.push(sel(e) + ` ${Math.round(r.width)}x${Math.round(r.height)}`)
  }
  for (const i of document.querySelectorAll('img')) if (!i.hasAttribute('alt')) out.noAlt.push(i.src.slice(-60))
  for (const e of document.querySelectorAll('input:not([type=hidden]):not([type=submit]):not([type=button]), select, textarea')) {
    if (!visible(e)) continue
    const has = e.getAttribute('aria-label') || e.getAttribute('aria-labelledby') || (e.labels && e.labels.length) || e.closest('label') || e.title
    if (!has) out.noLabel.push(sel(e) + ' placeholder=' + (e.placeholder || ''))
  }
  for (const e of document.querySelectorAll('a[href], button, [role=button]')) {
    if (!visible(e)) continue
    if (!(e.textContent.trim() || e.getAttribute('aria-label') || e.getAttribute('title') || e.querySelector('img[alt]:not([alt=""])'))) out.noName.push(sel(e))
  }
  // 對比
  const parse = c => { const m = c.match(/rgba?\(([^)]+)\)/); if (!m) return null; const p = m[1].split(/[ ,\/]+/).filter(Boolean).map(Number); return { r: p[0], g: p[1], b: p[2], a: p[3] ?? 1 } }
  const lum = ({ r, g, b }) => { const f = v => { v /= 255; return v <= .03928 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4 }; return .2126 * f(r) + .7152 * f(g) + .0722 * f(b) }
  const bgOf = e => { let layers = []; for (let n = e; n; n = n.parentElement) { const c = parse(getComputedStyle(n).backgroundColor); if (c && c.a > 0) { layers.push(c); if (c.a >= 1) break } } let base = { r: 255, g: 255, b: 255 }; if (!layers.length || layers[layers.length - 1].a < 1) base = getComputedStyle(document.documentElement).colorScheme.includes('dark') || matchMedia('(prefers-color-scheme: dark)').matches ? { r: 14, g: 14, b: 14 } : base; for (const l of layers.reverse()) base = { r: l.r * l.a + base.r * (1 - l.a), g: l.g * l.a + base.g * (1 - l.a), b: l.b * l.a + base.b * (1 - l.a) }; return base }
  const seen = new Set()
  for (const e of document.querySelectorAll('body *')) {
    if (!visible(e)) continue
    const own = [...e.childNodes].some(n => n.nodeType === 3 && n.textContent.trim())
    if (!own) continue
    const cs = getComputedStyle(e)
    const fg0 = parse(cs.color); if (!fg0) continue
    const bg = bgOf(e)
    const fg = { r: fg0.r * fg0.a + bg.r * (1 - fg0.a), g: fg0.g * fg0.a + bg.g * (1 - fg0.a), b: fg0.b * fg0.a + bg.b * (1 - fg0.a) }
    const L1 = lum(fg), L2 = lum(bg); const ratio = (Math.max(L1, L2) + .05) / (Math.min(L1, L2) + .05)
    const size = parseFloat(cs.fontSize), bold = parseInt(cs.fontWeight) >= 700
    const need = (size >= 24 || (size >= 18.66 && bold)) ? 3 : 4.5
    const key = sel(e) + cs.color + JSON.stringify(bg)
    if (ratio < need && !seen.has(key) && !(e.disabled || e.closest('[disabled]'))) { seen.add(key); out.lowContrast.push(`${sel(e)} ${ratio.toFixed(2)}:${need} fg=${cs.color} bg=rgb(${Math.round(bg.r)},${Math.round(bg.g)},${Math.round(bg.b)})`) }
  }
  for (const k of ['truncated', 'smallTargets', 'noAlt', 'noLabel', 'lowContrast', 'noName']) out[k] = [...new Set(out[k])].slice(0, 25)
  return out
}

/** Tab 走一輪，檢查聚焦元素是否有可見焦點樣式（outline 或 box-shadow 與非聚焦狀態不同） */
export async function focusAudit(page, max = 40) {
  await page.evaluate(() => { document.activeElement?.blur?.(); window.scrollTo(0, 0) })
  const res = []
  for (let i = 0; i < max; i++) {
    await page.keyboard.press('Tab')
    const r = await page.evaluate(() => {
      const e = document.activeElement
      if (!e || e === document.body) return null
      const cs = getComputedStyle(e)
      const outline = cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0
      const shadow = cs.boxShadow !== 'none'
      const rr = e.getBoundingClientRect()
      return { el: e.tagName.toLowerCase() + (e.getAttribute('aria-label') ? `[${e.getAttribute('aria-label')}]` : `「${(e.textContent || e.value || '').trim().slice(0, 16)}」`), outline, shadow, outlineColor: cs.outlineColor, w: Math.round(rr.width), visible: rr.width > 0 && rr.height > 0 }
    })
    if (!r) break
    res.push(r)
  }
  return res
}
