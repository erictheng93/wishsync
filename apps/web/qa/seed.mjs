// 種子資料：A(創建者)、B(另一創建者)、S(staff)、各種狀態清單與訪客。輸出 out/seed.json
import { registerUser, createList, guestClaim, sessionCookies, sql, q, API, OUT, uid, randomUUID, pwRequest, WEB } from './lib.mjs'
import { writeFileSync } from 'node:fs'

const A = await registerUser({ name: '小愛', tag: 'a' })
const B = await registerUser({ name: '阿B', tag: 'b' })
const S = await registerUser({ name: '管理員', tag: 's', staff: true })
const active = await createList(A, { title: '小愛的待產清單', showNames: true, items: [{ title: '奶瓶組', qty: 3, extra: { brand: 'Pigeon', spec: '240ml', product_url: 'https://example.com/p/1', unit_price_amount: 899 } }, { title: '嬰兒帽', qty: 1 }, { title: '很長很長很長很長很長很長很長很長很長很長很長很長很長很長很長很長很長很長的品項名稱ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789', qty: 2 }] })
const draft = await createList(A, { title: '草稿清單', publish: false })
const closed = await createList(A, { title: '已結束清單', items: [{ title: '結束品項', qty: 2 }] })
{ const cur = await (await A.api.get(`wishlists/${closed.id}`)).json(); await A.api.patch(`wishlists/${closed.id}`, { data: { status: 'closed', expected_updated_at: cur.wishlist.updated_at } }) }
const surprise = await createList(A, { title: '驚喜生日清單', surprise: true, items: [{ title: '驚喜禮物', qty: 2 }] })
const hidden = await createList(B, { title: '將被下架的清單', items: [{ title: '違規品項', qty: 1 }] })
const other = await createList(B, { title: 'B 的清單', items: [{ title: 'B 的品項', qty: 2 }] })
// 訪客認領
const g1 = await guestClaim(active.items[0].id, 1, '阿明', { note: '我會買 Pigeon', contact: 'line:aming' })
const g2 = await guestClaim(surprise.items[0].id, 1, '驚喜客')
const g3 = await guestClaim(other.items[0].id, 1, 'B 的訪客')
// 檢舉 + 下架
const rep = await pwRequest.newContext({ baseURL: API + '/api/v1/', extraHTTPHeaders: { Origin: WEB } })
const r = await rep.post(`public/wishlists/${hidden.slug}/reports`, { data: { reason: 'spam', detail: 'QA 測試檢舉' } })
console.log('report', r.status(), await r.text())
const wl = JSON.stringify
const seed = {
  A: { email: A.email, name: A.name, cookies: await sessionCookies(A) },
  B: { email: B.email, name: B.name, cookies: await sessionCookies(B) },
  S: { email: S.email, name: S.name, cookies: await sessionCookies(S) },
  active, draft, closed, surprise, hidden, other,
  g1: { token: g1.token, claimId: g1.claimId }, g2: { token: g2.token, claimId: g2.claimId }, g3: { token: g3.token, claimId: g3.claimId },
}
// 下架 hidden（staff）
const Sctx = await pwRequest.newContext({ baseURL: API + '/api/v1/', extraHTTPHeaders: { Origin: WEB } })
await Sctx.storageState()
const mod = await S.api.patch(`admin/wishlists/${hidden.id}/moderation`, { data: { moderation_status: 'hidden', reason: 'QA 下架' } })
console.log('moderate', mod.status(), await mod.text())
writeFileSync(OUT + '/seed.json', JSON.stringify(seed, null, 1))
console.log('seeded', active.slug, sql('select count(*) from wishlists'))
