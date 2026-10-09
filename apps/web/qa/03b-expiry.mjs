import { chromium, newCtx, watch, registerUser, createList, guestClaim, guestApi, sessionCookies, sql, q, log, OUT, API, WEB, uid, sleep, randomUUID, waitForMail, pwRequest, PASSWORD } from './lib.mjs'
import { writeFileSync } from 'node:fs'
writeFileSync(OUT + '/03b.jsonl', '')
const browser = await chromium.launch()
const L = (tag, o) => { console.log(tag, JSON.stringify(o).slice(0, 300)); log('03b.jsonl', { tag, ...o }) }
const step = async (tag, fn) => { try { await fn() } catch (e) { L(tag + ':SCRIPT-ERROR', { e: String(e).slice(0, 300) }) } }
sql('delete from rate_limits')
const openCard = async (page, name) => page.locator('li.g-card', { hasText: name }).getByRole('button', { name: '我要送' }).click()

// ---- S7 逾期認領 ----
await step('S7', async () => {
  const U = await registerUser({ tag: 's7' }); const T = await createList(U, { title: 'S7 清單', extra: { claim_ttl_hours: 1 }, items: [{ title: '限時品項', qty: 6 }] })
  const g = await guestClaim(T.items[0].id, 2, '逾期客'); const out = { expiresAt: g.body.claim.expires_at }
  sql(`update claims set expires_at = now() - interval '1 minute' where id='${g.claimId}'`)
  const pub = async () => (await (await (await guestApi()).get(`public/wishlists/${T.slug}`)).json()).items[0].qty_claimed
  out.claimedBeforeJob = await pub()
  // 另一訪客此時想認領：應該被擋（job 尚未跑）
  const g2 = await guestClaim(T.items[0].id, 1, '想搶的人'); out.otherGuestWhileExpiredUnreleased = g2.status
  // 逾期但尚未釋放時，原訪客改為已購買 → 是否「救回」
  const ga = g.api; const hdr = { 'X-Guest-Token': g.token }
  const buy = await ga.patch(`claims/${g.claimId}`, { data: { status: 'purchased' }, headers: hdr }); out.purchaseAfterExpiry = buy.status()
  out.expiresAfterPurchase = sql(`select status||' '||coalesce(expires_at::text,'null') from claims where id='${g.claimId}'`)
  // 再做一筆：讓背景 job 釋放，等最多 5.5 分鐘
  const g3 = await guestClaim(T.items[0].id, 1, '等 job 的人'); out.g3 = g3.status
  if (g3.claimId) sql(`update claims set expires_at = now() - interval '1 minute' where id='${g3.claimId}'`)
  const t0 = Date.now(); let released = false
  while (Date.now() - t0 < 330000) { if (sql(`select status from claims where id='${g3.claimId}'`) === 'expired') { released = true; break } await sleep(10000) }
  out.jobReleasedAfterSec = released ? Math.round((Date.now() - t0) / 1000) : 'not within 330s'
  out.statusAfter = sql(`select status from claims where id='${g3.claimId}'`); out.claimedAfter = await pub()
  out.audit = sql(`select count(*) from audit_logs where action='claim.expire' and entity_id='${g3.claimId}'`)
  const me = await ga.get('guest/me', { headers: { 'X-Guest-Token': g3.token } }); out.guestMeShowsExpired = JSON.stringify((await (await (await guestApi()).get('guest/me', { headers: { 'X-Guest-Token': g3.token } })).json()).claims?.map(c => c.claim.status))
  // 逾期後想改數量
  const up = await (await guestApi()).patch(`claims/${g3.claimId}`, { data: { qty: 1 }, headers: { 'X-Guest-Token': g3.token } }); out.patchExpiredClaim = up.status() + ' ' + (await up.text()).slice(0, 100)
  L('S7', out)
})

await browser.close()
