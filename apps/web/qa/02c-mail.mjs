import { registerUser, createList, guestClaim, waitForMail, sql, q, log, OUT, pwRequest, MAILPIT, sleep } from './lib.mjs'
const U = await registerUser({ name: '郵件<b>測試</b>', tag: 'ml' })
const titles = { crlf: 'Hi\r\nBcc: evil@example.com\r\nX-Injected: 1', xss: '<script>alert(1)</script>"><img src=x>', long: '長'.repeat(100), bidi: 'abc‮def' }
for (const [k, t] of Object.entries(titles)) {
  const L = await createList(U, { title: t, items: [{ title: 'x', qty: 3 }] })
  const g = await guestClaim(L.items[0].id, 1, '客<i>人</i>', { email: `guest-${k}-${Date.now()}@example.com`, note: '<b>n</b>' })
  console.log(k, 'claim', g.status)
  await sleep(6000)
  const api = await pwRequest.newContext()
  const list = (await (await api.get(`${MAILPIT}/api/v1/search`, { params: { query: `to:${U.email}` } })).json()).messages
  const gl = (await (await api.get(`${MAILPIT}/api/v1/search`, { params: { query: `to:guest-${k}-*` } })).json()).messages
  for (const m of [...list, ...gl].slice(0, 8)) {
    const full = await (await api.get(`${MAILPIT}/api/v1/message/${m.ID}`)).json()
    const hdr = await (await api.get(`${MAILPIT}/api/v1/message/${m.ID}/headers`)).json()
    log('02c.jsonl', { k, to: m.To?.[0]?.Address, subject: m.Subject, bcc: m.Bcc?.length, injected: hdr['X-Injected'], textHead: full.Text.slice(0, 160) })
  }
}
console.log(sql(`select kind,status,last_error,count(*) from notifications group by 1,2,3`))
