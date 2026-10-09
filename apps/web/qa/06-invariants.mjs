import { registerUser, createList, guestClaim, guestApi, sql, q, log, OUT, randomUUID, API, WEB } from './lib.mjs'
const L = (tag, o) => { console.log(tag, JSON.stringify(o).slice(0, 600)); log('06.jsonl', { tag, ...o }) }
sql('delete from rate_limits')
const U = await registerUser({ tag: 'inv' }), S = await registerUser({ tag: 'invs', staff: true })
// 1 併發超賣：40 個匿名同時認領 qty=5 的品項
const T = await createList(U, { title: '併發', items: [{ title: '搶', qty: 5 }, { title: '二', qty: 3 }] })
const res = await Promise.all(Array.from({ length: 40 }, async (_, i) => { const a = await guestApi(); const r = await a.post(`items/${T.items[0].id}/claims`, { data: { qty: 1, display_name: 'p' + i }, headers: { 'Idempotency-Key': randomUUID() } }); return r.status() }))
L('concurrent', { s201: res.filter(x => x === 201).length, s409: res.filter(x => x === 409).length, other: res.filter(x => ![201, 409].includes(x)), dbClaimed: sql(`select qty_claimed from wishlist_items where id='${T.items[0].id}'`) })
// 同一 guest 並發兩個 qty 修改
const g = await guestClaim(T.items[1].id, 1, '改量客'); const h = { 'X-Guest-Token': g.token }
const ups = await Promise.all([2, 3, 3, 1, 2, 3].map(async qty => (await (await guestApi()).patch(`claims/${g.claimId}`, { data: { qty }, headers: h })).status()))
L('concurrent-patch', { statuses: ups, claimed: sql(`select qty_claimed from wishlist_items where id='${T.items[1].id}'`), claimQty: sql(`select qty from claims where id='${g.claimId}'`) })
// 2 強制刪除有認領的品項
const F = await createList(U, { title: '強刪', items: [{ title: '將被刪', qty: 4 }] }); const gf = await guestClaim(F.items[0].id, 2, '被取消客')
const noforce = (await U.api.delete(`items/${F.items[0].id}`)).status(); const force = (await U.api.delete(`items/${F.items[0].id}?force=true`)).status()
L('force-delete', { noforce, force, claimStatus: sql(`select status from claims where id='${gf.claimId}'`), itemQtyClaimedAfter: sql(`select qty_claimed from wishlist_items where id='${F.items[0].id}'`), auditForCancel: sql(`select count(*) from audit_logs where entity_id='${gf.claimId}' and action<>'claim.create'`), guestMe: JSON.stringify((await (await (await guestApi()).get('guest/me', { headers: { 'X-Guest-Token': gf.token } })).json()).claims.map(c => c.claim.status)) })
// 3 owner 取消認領 + 標記 delivered
const O = await createList(U, { title: '擁有者操作', items: [{ title: '甲', qty: 3 }] }); const go = await guestClaim(O.items[0].id, 2, '甲客'); const go2 = await guestClaim(O.items[0].id, 1, '乙客')
L('owner-actions', { deliver: (await U.api.patch(`claims/${go.claimId}`, { data: { status: 'delivered' } })).status(), cancel: (await U.api.delete(`claims/${go2.claimId}`)).status(), claimed: sql(`select qty_claimed from wishlist_items where id='${O.items[0].id}'`), audits: sql(`select string_agg(action||':'||actor_type, ',') from audit_logs where entity_id in ('${go.claimId}','${go2.claimId}')`) })
// 4 檢舉處理 + 稽核
const rep = await (await guestApi()).post(`public/wishlists/${O.slug}/reports`, { data: { reason: 'scam', detail: 'inv' } }); const rid = (await rep.json()).id
const hr = await S.api.patch(`admin/reports/${rid}`, { data: { status: 'actioned' } }); L('report-handle', { status: hr.status(), body: (await hr.text()).slice(0, 120), audits: sql(`select string_agg(action,',') from audit_logs where entity_id='${rid}'`) })
const mod = await S.api.patch(`admin/wishlists/${O.id}/moderation`, { data: { moderation_status: 'hidden', reason: 'inv' } }); const un = await S.api.patch(`admin/wishlists/${O.id}/moderation`, { data: { moderation_status: 'ok' } })
L('moderate-restore', { hide: mod.status(), restore: un.status(), audit: sql(`select count(*) from audit_logs where entity_id='${O.id}' and action='wishlist.moderate'`) })
const flag = await S.api.put('admin/system-flags/read_only', { data: { value: true } }); const ro = await (await guestApi()).post(`items/${T.items[1].id}/claims`, { data: { qty: 1, display_name: 'ro' }, headers: { 'Idempotency-Key': randomUUID() } }); const flag2 = await S.api.put('admin/system-flags/read_only', { data: { value: false } })
L('read-only', { setOn: flag.status(), claimDuring: ro.status(), setOff: flag2.status(), flagAudit: sql(`select count(*) from audit_logs where action like '%flag%'`) })
// 5 帳號刪除
const D = await registerUser({ tag: 'del' }); const DL = await createList(D, { title: '待刪帳號清單', items: [{ title: 'z', qty: 2 }] }); const gd = await guestClaim(DL.items[0].id, 1, '客')
const dm = await D.api.delete('me'); L('delete-account', { status: dm.status(), body: (await dm.text()).slice(0, 100), me: (await D.api.get('me')).status(), publicPage: (await (await guestApi()).get(`public/wishlists/${DL.slug}`)).status(), claimStatus: sql(`select status from claims where id='${gd.claimId}'`), claimedQty: sql(`select qty_claimed from wishlist_items where id='${DL.items[0].id}'`), listDeleted: sql(`select deleted_at is not null from wishlists where id='${DL.id}'`), email: sql(`select email from users where id=(select owner_id from wishlists where id='${DL.id}')`), audit: sql(`select string_agg(action,',') from audit_logs where actor_id=(select owner_id from wishlists where id='${DL.id}')`), sessions: sql(`select count(*) from sessions where user_id=(select owner_id from wishlists where id='${DL.id}') and revoked_at is null`), guestAfter: (await (await guestApi()).patch(`claims/${gd.claimId}`, { data: { qty: 1 }, headers: { 'X-Guest-Token': gd.token } })).status() })
// 全面不變量
const inv = {
  mismatch_all_items: sql(`select count(*) from wishlist_items i where qty_claimed <> coalesce((select sum(qty) from claims c where c.item_id=i.id and c.status in ('reserved','purchased','delivered')),0)`),
  mismatch_live_items: sql(`select count(*) from wishlist_items i where deleted_at is null and qty_claimed <> coalesce((select sum(qty) from claims c where c.item_id=i.id and c.status in ('reserved','purchased','delivered')),0)`),
  overclaimed: sql(`select count(*) from wishlist_items where qty_claimed>qty_needed or qty_claimed<0`),
  orphanClaims: sql(`select count(*) from claims c where (guest_id is null and user_id is null) or not exists (select 1 from wishlist_items i where i.id=c.item_id)`),
  orphanItems: sql(`select count(*) from wishlist_items i where not exists (select 1 from wishlists w where w.id=i.wishlist_id)`),
  claimsWithoutCreateAudit: sql(`select count(*) from claims c where not exists (select 1 from audit_logs a where a.entity_id=c.id and a.action='claim.create')`),
  idemWithToken: sql(`select count(*) from idempotency_keys where response_body::text ilike '%guest_token%'`),
  plaintextTokensInGuests: sql(`select count(*) from guests where length(guest_token_hash)<>32`),
  dupActive: sql(`select count(*) from (select item_id, coalesce(guest_id,user_id), count(*) from claims where status in ('reserved','purchased','delivered') group by 1,2 having count(*)>1) x`),
  auditActions: sql(`select string_agg(action||'='||n, ', ') from (select action, count(*) n from audit_logs group by 1 order by 1) x`),
}
L('invariants', inv)
