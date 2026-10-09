import { expect, request as pwRequest, type APIRequestContext, type BrowserContext, type Page } from '@playwright/test'
import { randomBytes } from 'node:crypto'
import { API, E2E_DB, MAILPIT } from './support/env'
// @ts-expect-error 純 JS 模組（也給 reset-db.mjs 用）
import { psql } from './support/psql.mjs'

export const PASSWORD = 'e2e-Passw0rd!'
export const uid = () => randomBytes(5).toString('hex')
export const newEmail = (tag = 'u') => `e2e-${tag}-${uid()}@example.com`

// ---------- Mailpit ----------
type Msg = { ID: string; Subject: string; To: { Address: string }[] }
/** 等到寄給 to 的信（可選 subject 過濾），回傳純文字內文。Mailpit 是共用的，所以一律以唯一收件人過濾。 */
export async function waitForMail(to: string, opts: { subject?: RegExp; after?: number } = {}): Promise<{ id: string; subject: string; text: string }> {
  const api = await pwRequest.newContext()
  try {
    let found: Msg | undefined
    await expect.poll(async () => {
      const r = await api.get(`${MAILPIT}/api/v1/search`, { params: { query: `to:${to}` } })
      const list: Msg[] = (await r.json()).messages ?? []
      found = list.find(m => !opts.subject || opts.subject.test(m.Subject))
      return !!found
    }, { message: `等不到寄給 ${to} 的信`, timeout: 20_000, intervals: [200, 300, 500, 1000] }).toBe(true)
    const m = await (await api.get(`${MAILPIT}/api/v1/message/${found!.ID}`)).json()
    return { id: found!.ID, subject: found!.Subject, text: m.Text as string }
  } finally { await api.dispose() }
}
/** 取得 OTP；同一 email 後來又寄一封時，傳 skip 排除舊的驗證碼 */
export async function waitForOtp(email: string, subject?: RegExp): Promise<string> {
  const m = await waitForMail(email, { subject })
  const code = m.text.match(/(\d{6})/)?.[1]
  if (!code) throw new Error(`信件內沒有驗證碼：${m.text}`)
  return code
}

// ---------- DB（psql 子行程，不新增 npm 依賴）----------
const q = (s: string) => s.replace(/'/g, "''")
export const sql = (stmt: string): string => psql(E2E_DB, stmt).trim()
export function makeStaff(email: string) {
  const n = sql(`UPDATE users SET is_staff = true WHERE email = '${q(email)}' RETURNING 1`)
  if (n.split('\n')[0] !== '1') throw new Error(`找不到使用者 ${email}`)
}
/** OTP 共用「每 email 60 秒間隔」限流；需要在同一個 email 連續取第二封信時把舊紀錄往前推 */
export const skipOtpCooldown = (email: string) => sql(`UPDATE otp_challenges SET created_at = created_at - interval '5 minutes' WHERE email = '${q(email)}'`)

// ---------- API（創建者）----------
export const apiCtx = (): Promise<APIRequestContext> => pwRequest.newContext({ baseURL: API + '/api/v1/', extraHTTPHeaders: { Origin: 'http://localhost:3012' } })
const must = async (r: import('@playwright/test').APIResponse, what: string) => {
  if (!r.ok()) throw new Error(`${what} 失敗：${r.status()} ${await r.text()}`)
  return r
}

export type User = { email: string; password: string; name: string; api: APIRequestContext }
/** 以 API 註冊（OTP 從 Mailpit 取）並登入；回傳帶 session cookie 的 request context */
export async function registerUser(opts: { name?: string; tag?: string } = {}): Promise<User> {
  const email = newEmail(opts.tag), name = opts.name ?? `創建者${uid().slice(0, 4)}`
  const api = await apiCtx()
  await must(await api.post('auth/register', { data: { email, password: PASSWORD, display_name: name } }), '註冊')
  const code = await waitForOtp(email)
  await must(await api.post('auth/register/verify', { data: { email, code } }), '驗證註冊')
  return { email, password: PASSWORD, name, api }
}

export type ListOpts = { title?: string; surprise?: boolean; showNames?: boolean; items?: { title: string; qty?: number }[]; publish?: boolean }
export type List = { id: string; slug: string; title: string; items: { id: string; title: string }[] }
/** 建立清單 + 品項 + （預設）發佈 */
export async function createList(user: User, o: ListOpts = {}): Promise<List> {
  const title = o.title ?? `E2E 清單 ${uid()}`
  const body: any = { type: 'registry', title, visibility: 'link', surprise_mode: !!o.surprise, show_claimer_names: !!o.showNames }
  if (o.surprise) body.event_date = new Date(Date.now() + 30 * 864e5 + 8 * 36e5).toISOString().slice(0, 10)
  const w = await (await must(await user.api.post('wishlists', { data: body }), '建立清單')).json()
  const items = []
  for (const it of o.items ?? [{ title: '奶瓶', qty: 1 }]) {
    const r = await (await must(await user.api.post(`wishlists/${w.id}/items`, { data: { title: it.title, qty_needed: it.qty ?? 1, priority: 'medium', funding_mode: 'quantity' } }), '新增品項')).json()
    items.push({ id: r.id as string, title: it.title })
  }
  let slug = w.slug as string | undefined
  if (o.publish !== false) {
    const cur = await (await must(await user.api.get(`wishlists/${w.id}`), '讀取清單')).json()
    const p = await (await must(await user.api.patch(`wishlists/${w.id}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } }), '發佈')).json()
    slug = p.slug ?? cur.wishlist.slug
  }
  slug ??= (await (await user.api.get(`wishlists/${w.id}`)).json()).wishlist.slug
  return { id: w.id, slug: slug!, title, items }
}

/** 把 API context 的 session cookie 帶進瀏覽器 context（跳過 UI 登入） */
export async function loginBrowser(ctx: BrowserContext, user: User) {
  await ctx.addCookies((await user.api.storageState()).cookies)
}

/** 用 UI 登入（密碼） */
export async function uiLogin(page: Page, email: string, password: string) {
  await page.goto('/login')
  await page.getByLabel('Email').fill(email)
  await page.getByLabel('密碼').fill(password)
  await page.getByRole('button', { name: '登入', exact: true }).click()
}

// ---------- 訪客 ----------
/** 以 API 作為新訪客認領；回傳 guest token 與 claim */
export async function guestClaim(itemId: string, qty = 1, name = `訪客${uid().slice(0, 4)}`) {
  const api = await pwRequest.newContext({ baseURL: API + '/api/v1/' })
  try {
    const r = await must(await api.post(`items/${itemId}/claims`, { data: { qty, display_name: name }, headers: { 'Idempotency-Key': crypto.randomUUID() } }), '訪客認領')
    return await r.json()
  } finally { await api.dispose() }
}

/** 收集頁面的 console error 與失敗請求（4xx/5xx），給煙霧測試用 */
export function watchProblems(page: Page, ignore: RegExp[] = []) {
  const problems: string[] = []
  // 預期內：未登入 / 無訪客身分時，前端會探測 /me、/guest/me，401 是正常答案（瀏覽器仍會印一行 console error）
  const expected = [/^401 .*\/api\/v1\/(guest\/)?me(\?|$)/, /^Failed to load resource: .*status of 401/]
  const skip = (s: string) => [...expected, ...ignore].some(r => r.test(s))
  page.on('console', m => { if (m.type() === 'error' && !skip(m.text())) problems.push(`console: ${m.text()}`) })
  page.on('pageerror', e => problems.push(`pageerror: ${e.message}`))
  page.on('response', r => { if (r.status() >= 400 && !skip(`${r.status()} ${r.url()}`)) problems.push(`${r.status()} ${r.url()}`) })
  return problems
}

/** 透過 UI 認領品項（未登入訪客）。回傳後停在成功畫面 */
export async function claimViaUi(page: Page, itemTitle: string, name: string, qty = 1) {
  const sheet = page.getByRole('dialog')
  await page.locator('li.g-card', { hasText: itemTitle }).getByRole('button', { name: '我要送' }).click()
  await sheet.getByLabel('你的暱稱（必填）').fill(name)
  for (let i = 1; i < qty; i++) await sheet.getByRole('button', { name: '增加' }).click()
  await sheet.getByRole('button', { name: '確認認領' }).click()
  await expect(page.getByRole('heading', { name: '✓ 認領成功！' })).toBeVisible()
}
