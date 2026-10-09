import { registerUser, createList, guestClaim, guestApi, sql, log, randomUUID } from './lib.mjs'
const L = (tag, o) => { console.log(tag, JSON.stringify(o).slice(0, 700)); log('06.jsonl', { tag, ...o }) }
const O = await registerUser({ tag: 'dow' }), D = await registerUser({ tag: 'del2' })
const OL = await createList(O, { title: '別人的清單', items: [{ title: 'q', qty: 3 }] }), DL = await createList(D, { title: '待刪帳號清單', items: [{ title: 'z', qty: 2 }] })
const gd = await guestClaim(DL.items[0].id, 1, '客')
const mine = await D.api.post(`items/${OL.items[0].id}/claims`, { data: { qty: 2 }, headers: { 'Idempotency-Key': randomUUID() } }); const cid = (await mine.json()).claim?.id
const del = await D.api.delete('me', { data: { confirm: 'DELETE' } })
const wid = DL.id
L('delete-account', { status: del.status(), me: (await D.api.get('me')).status(), publicPage: (await (await guestApi()).get(`public/wishlists/${DL.slug}`)).status(), guestClaimStatusOnArchivedList: sql(`select status from claims where id='${gd.claimId}'`), userClaimOnOthersList: sql(`select status||' qty='||qty||' name='||claimer_name||' expires='||coalesce(expires_at::text,'null') from claims where id='${cid}'`), othersItemClaimed: sql(`select qty_claimed from wishlist_items where id='${OL.items[0].id}'`), audit: sql(`select string_agg(action,',') from audit_logs where actor_id=(select owner_id from wishlists where id='${wid}')`), guestPatchAfter: (await (await guestApi()).patch(`claims/${gd.claimId}`, { data: { qty: 1 }, headers: { 'X-Guest-Token': gd.token } })).status(), ownerSeesDeletedUserClaim: JSON.stringify((await (await O.api.get(`wishlists/${OL.id}/dashboard`)).json()).claims.map(c => c.claimer_name)) })
L('invariants-after', { mismatchLive: sql(`select count(*) from wishlist_items i where deleted_at is null and qty_claimed <> coalesce((select sum(qty) from claims c where c.item_id=i.id and c.status in ('reserved','purchased','delivered')),0)`) })
