import { chromium, newCtx, watch, registerUser, createList, guestClaim, guestApi, sessionCookies, sql, q, log, OUT, API, WEB, uid, sleep, randomUUID, waitForMail, pwRequest, PASSWORD } from './lib.mjs'
import { writeFileSync } from 'node:fs'
if (!process.env.ONLY) writeFileSync(OUT + '/03.jsonl', '')
const browser = await chromium.launch()
const L = (tag, o) => { console.log(tag, JSON.stringify(o).slice(0, 300)); log('03.jsonl', { tag, ...o }) }
const step = async (tag, fn) => { if (process.env.ONLY && !process.env.ONLY.split(',').includes(tag)) return; try { await fn() } catch (e) { L(tag + ':SCRIPT-ERROR', { e: String(e).slice(0, 300) }) } }
sql('delete from rate_limits')
const openSettings = async (page) => { await page.locator('summary', { hasText: '清單設定' }).click(); await page.waitForTimeout(200) }
const openCard = async (page, name) => page.locator('li.g-card', { hasText: name }).getByRole('button', { name: '我要送' }).click()

// ---- S1 history / reload / keyboard in claim sheet ----
await step('S1', async () => {
  const U = await registerUser({ tag: 's1' }); const T = await createList(U, { title: 'S1 清單', items: [{ title: '品項一', qty: 3 }] })
  const ctx = await newCtx(browser); const page = await ctx.newPage(); const sink = []; watch(page, 's1', sink)
  await page.goto('/'); await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' })
  await openCard(page, '品項一'); const d = page.getByRole('dialog')
  const focusInit = await page.evaluate(() => document.activeElement?.tagName + '#' + (document.activeElement?.id || ''))
  await d.getByLabel('你的暱稱（必填）').fill('歷史哥')
  // Tab 走 12 次，看焦點是否跑出 dialog
  let escaped = 0; for (let i = 0; i < 14; i++) { await page.keyboard.press('Tab'); if (!(await page.evaluate(() => !!document.activeElement?.closest('[role=dialog]')))) escaped++ }
  L('S1.focus-trap', { initialFocus: focusInit, tabsOutsideDialog: escaped, of: 14 })
  await page.keyboard.press('Escape'); await page.waitForTimeout(300)
  L('S1.escape-closes', { dialogStillOpen: await page.getByRole('dialog').isVisible().catch(() => false) })
  await page.getByRole('button', { name: '取消' }).click(); await openCard(page, '品項一'); await page.getByRole('dialog').getByLabel('你的暱稱（必填）').fill('歷史哥')
  await page.goBack(); await page.waitForTimeout(500)
  L('S1.back-with-sheet-open', { url: page.url().replace(WEB, ''), sheetOpen: await page.getByRole('dialog').isVisible().catch(() => false) })
  await page.goForward(); await page.waitForTimeout(800)
  await openCard(page, '品項一'); await page.getByRole('dialog').getByLabel('你的暱稱（必填）').fill('歷史哥')
  await page.reload({ waitUntil: 'networkidle' })
  L('S1.reload-with-sheet-open', { sheetOpen: await page.getByRole('dialog').isVisible().catch(() => false) })
  // backdrop click loses typed input
  await openCard(page, '品項一'); await page.getByRole('dialog').getByLabel('你的暱稱（必填）').fill('點背景哥')
  await page.mouse.click(195, 60); await page.waitForTimeout(300)
  L('S1.backdrop-click', { sheetOpen: await page.locator('.g-sheet').isVisible().catch(() => false) })
  // 成功畫面後 reload / back
  await openCard(page, '品項一'); await page.getByRole('dialog').getByLabel('你的暱稱（必填）').fill('歷史哥'); await page.getByRole('button', { name: '確認認領' }).click()
  await page.getByRole('heading', { name: '✓ 認領成功！' }).waitFor()
  const succFocus = await page.evaluate(() => document.activeElement?.tagName)
  let succEsc = 0; for (let i = 0; i < 6; i++) { await page.keyboard.press('Tab'); if (!(await page.evaluate(() => !!document.activeElement?.closest('[role=dialog]')))) succEsc++ }
  await page.keyboard.press('Escape'); await page.waitForTimeout(300)
  L('S1.success-overlay-a11y', { focusOnOpen: succFocus, tabsOutside: succEsc, of: 6, stillOpenAfterEsc: await page.getByRole('heading', { name: '✓ 認領成功！' }).isVisible() })
  await page.goBack(); await page.waitForTimeout(500); L('S1.back-from-success', { url: page.url().replace(WEB, '') })
  await page.goForward(); await page.waitForTimeout(800)
  await page.reload({ waitUntil: 'networkidle' }); L('S1.reload-after-success', { mineShown: await page.getByText('你已認領 1 件').isVisible() })
  L('S1.console', { sink: sink.filter(s => !/401/.test(s.text)).slice(0, 5) })
  await ctx.close()
})

// ---- S1b 登入流程：back / authed visiting /login / redirect ----
await step('S1b', async () => {
  const U = await registerUser({ tag: 's1b' })
  const ctx = await newCtx(browser); const page = await ctx.newPage()
  await page.goto('/dashboard', { waitUntil: 'networkidle' }); L('S1b.unauth-dashboard', { url: page.url().replace(WEB, '') })
  await page.getByLabel('Email').fill(U.email); await page.getByLabel('密碼').fill(PASSWORD); await page.getByRole('button', { name: '登入', exact: true }).click()
  await page.waitForURL(u => u.pathname === '/dashboard'); L('S1b.after-login', { url: page.url().replace(WEB, '') })
  await page.goBack(); await page.waitForTimeout(800); L('S1b.back-after-login', { url: page.url().replace(WEB, ''), hasLoginForm: await page.getByLabel('密碼').isVisible().catch(() => false) })
  await page.goto('/login?redirect=https://evil.example.com/', { waitUntil: 'networkidle' }); await page.waitForTimeout(500); L('S1b.open-redirect-authed', { url: page.url() })
  await ctx.clearCookies(); await page.goto('/login?redirect=https://evil.example.com/', { waitUntil: 'networkidle' })
  await page.getByLabel('Email').fill(U.email); await page.getByLabel('密碼').fill(PASSWORD); await page.getByRole('button', { name: '登入', exact: true }).click(); await page.waitForTimeout(1500)
  L('S1b.open-redirect-login', { url: page.url() })
  await page.goto('/login?redirect=//evil.example.com/x', { waitUntil: 'networkidle' }); await page.waitForTimeout(500); L('S1b.open-redirect2-authed', { url: page.url() })
  await page.goto('/login?redirect=javascript:alert(1)', { waitUntil: 'networkidle' }); await page.waitForTimeout(500); L('S1b.js-redirect-authed', { url: page.url() })
  await ctx.close()
})

// ---- S2 多分頁登入登出 ----
await step('S2', async () => {
  const U = await registerUser({ tag: 's2' }); const T = await createList(U, { title: 'S2 清單', items: [{ title: 'x', qty: 1 }] })
  const ctx = await newCtx(browser, { cookies: await sessionCookies(U) })
  const p1 = await ctx.newPage(), p2 = await ctx.newPage(); const sink = []; watch(p2, 's2', sink)
  await p1.goto(`/lists/${T.id}/edit`, { waitUntil: 'networkidle' }); await p2.goto(`/lists/${T.id}/edit`, { waitUntil: 'networkidle' })
  await p1.goto('/settings', { waitUntil: 'networkidle' }); await p1.getByRole('button', { name: '登出' }).click(); await p1.waitForURL(/login/)
  // p2 仍顯示已登入頁面；做操作
  await openSettings(p2); await p2.getByLabel('清單名稱').first().fill('改了標題'); 
  await p2.getByRole('button', { name: '儲存設定' }).click().catch(() => {}); await p2.waitForTimeout(1200)
  L('S2.other-tab-after-logout-save', { url: p2.url().replace(WEB, ''), text: (await p2.innerText('body')).slice(0, 120).replace(/\n/g, '|') })
  await p2.screenshot({ path: OUT + '/shots/s2__tab2-after-logout.png' })
  // 反向：p1 在 /login，p2 登入後 p1 reload
  await p2.goto('/login?redirect=/dashboard', { waitUntil: 'networkidle' }); await p2.getByLabel('Email').fill(U.email); await p2.getByLabel('密碼').fill(PASSWORD); await p2.getByRole('button', { name: '登入', exact: true }).click(); await p2.waitForURL(u => u.pathname === '/dashboard')
  await p1.waitForTimeout(500); L('S2.login-tab-still-on-login', { p1url: p1.url().replace(WEB, '') })
  await p1.getByLabel('Email').fill(U.email); await p1.getByLabel('密碼').fill(PASSWORD); await p1.getByRole('button', { name: '登入', exact: true }).click(); await p1.waitForTimeout(1500)
  L('S2.second-login', { p1url: p1.url().replace(WEB, '') , sessions: sql(`select count(*) filter (where revoked_at is null) ||'/'|| count(*) from sessions s join users u on u.id=s.user_id where u.email='${q(U.email)}'`) })
  await ctx.close()
})

// ---- S3 session 過期 ----
await step('S3', async () => {
  const U = await registerUser({ tag: 's3' }); const T = await createList(U, { title: 'S3 清單', items: [{ title: '品項', qty: 2 }] })
  const ctx = await newCtx(browser, { cookies: await sessionCookies(U) }); const page = await ctx.newPage()
  await page.goto(`/lists/${T.id}/edit`, { waitUntil: 'networkidle' })
  await openSettings(page); await page.getByLabel('清單名稱').first().fill('S3 未儲存的修改標題')
  sql(`update sessions set expires_at = now() - interval '1 minute' where user_id=(select id from users where email='${q(U.email)}')`)
  await page.getByRole('button', { name: '儲存設定' }).click(); await page.waitForTimeout(1500)
  L('S3.save-after-expiry', { url: page.url().replace(WEB, ''), text: (await page.innerText('body')).slice(0, 160).replace(/\n/g, '|') })
  await page.screenshot({ path: OUT + '/shots/s3__expired-save.png' })
  // 重新登入後是否回到原頁
  await page.getByLabel('Email').fill(U.email).catch(() => {}); await page.getByLabel('密碼').fill(PASSWORD).catch(() => {}); await page.getByRole('button', { name: '登入', exact: true }).click().catch(() => {}); await page.waitForTimeout(1500)
  L('S3.relogin-redirect', { url: page.url().replace(WEB, ''), titleField: await page.getByLabel('清單名稱').first().inputValue().catch(() => null) })
  // 訪客頁：過期後 /s/ 頁應仍可開（匿名）
  sql(`update sessions set expires_at = now() - interval '1 minute' where user_id=(select id from users where email='${q(U.email)}')`)
  await page.goto('/dashboard', { waitUntil: 'networkidle' }); L('S3.dashboard-expired', { url: page.url().replace(WEB, '') })
  await ctx.close()
})

// ---- S4 清單狀態改變時，開著的頁面 ----
await step('S4', async () => {
  const U = await registerUser({ tag: 's4' }); const S = await registerUser({ tag: 's4s', staff: true })
  const mk = async (t) => createList(U, { title: t, items: [{ title: t + '品項', qty: 5 }, { title: t + '第二', qty: 5 }] })
  for (const kind of (process.env.S4KINDS ?? 'closed,archived,hidden,itemdeleted').split(',')) {
    const T = await mk('S4' + kind)
    const ctx = await newCtx(browser); const page = await ctx.newPage()
    await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' }); await page.waitForTimeout(500)
    await openCard(page, T.title + '品項'); const d = page.getByRole('dialog'); await d.getByLabel('你的暱稱（必填）').fill('開著頁面的客人')
    const cur = await (await U.api.get(`wishlists/${T.id}`)).json()
    if (kind === 'closed') await U.api.patch(`wishlists/${T.id}`, { data: { status: 'closed', expected_updated_at: cur.wishlist.updated_at } })
    if (kind === 'archived') await U.api.delete(`wishlists/${T.id}`)
    if (kind === 'hidden') await S.api.patch(`admin/wishlists/${T.id}/moderation`, { data: { moderation_status: 'hidden', reason: 'q' } })
    if (kind === 'itemdeleted') await U.api.delete(`items/${T.items[0].id}`)
    await page.waitForTimeout(kind === 'closed' ? 1500 : 500)
    await d.getByRole('button', { name: '確認認領' }).click(); await page.waitForTimeout(1200)
    const msg = await d.locator('.g-banner.err').innerText().catch(() => '(none)'); const succ = await page.getByRole('heading', { name: '✓ 認領成功！' }).isVisible()
    await page.screenshot({ path: `${OUT}/shots/s4__${kind}-claim.png` })
    // 重新整理後
    await page.reload({ waitUntil: 'networkidle' }).catch(() => {})
    L('S4.' + kind, { claimMsg: msg, claimSucceeded: succ, afterReload: (await page.innerText('body')).slice(0, 100).replace(/\n/g, '|'), banner: await page.locator('.g-banner').first().innerText().catch(() => '(none)') })
    await ctx.close()
  }
  // 創建者編輯頁開著，被下架 / 結束 / 封存
  for (const kind of ['hidden', 'closed', 'archived']) {
    const T = await mk('S4e' + kind)
    const ctx = await newCtx(browser, { cookies: await sessionCookies(U) }); const page = await ctx.newPage()
    await page.goto(`/lists/${T.id}/edit`, { waitUntil: 'networkidle' })
    if (kind === 'hidden') await S.api.patch(`admin/wishlists/${T.id}/moderation`, { data: { moderation_status: 'hidden', reason: '違規內容' } })
    if (kind === 'closed') { const cur = await (await U.api.get(`wishlists/${T.id}`)).json(); await U.api.patch(`wishlists/${T.id}`, { data: { status: 'closed', expected_updated_at: cur.wishlist.updated_at } }) }
    if (kind === 'archived') await U.api.delete(`wishlists/${T.id}`)
    await openSettings(page); await page.getByLabel('清單名稱').first().fill('改' + kind); await page.getByRole('button', { name: '儲存設定' }).click(); await page.waitForTimeout(1000)
    const m1 = (await page.locator('.c-err, [role=alert], .c-ok, [role=status]').allInnerTexts()).join('|')
    await page.getByRole('button', { name: '＋ 新增品項' }).click().catch(() => {}); await page.waitForTimeout(300)
    await page.getByLabel('名稱（必填）').first().fill('新增的品項').catch(() => {}); await page.getByRole('button', { name: '儲存品項' }).click().catch(() => {}); await page.waitForTimeout(1000)
    const m2 = (await page.locator('.c-err, [role=alert]').allInnerTexts()).join('|')
    await page.screenshot({ path: `${OUT}/shots/s4__owner-${kind}.png` })
    L('S4.owner-' + kind, { saveMsg: m1, addItemMsg: m2 })
    await ctx.close()
  }
})

// ---- S5 兩分頁編輯衝突 ----
await step('S5', async () => {
  const U = await registerUser({ tag: 's5' }); const T = await createList(U, { title: 'S5 清單', items: [{ title: '品項', qty: 2 }] })
  const ctx = await newCtx(browser, { cookies: await sessionCookies(U) }); const p1 = await ctx.newPage(), p2 = await ctx.newPage()
  for (const p of [p1, p2]) await p.goto(`/lists/${T.id}/edit`, { waitUntil: 'networkidle' })
  await openSettings(p1); await p1.getByLabel('清單名稱').first().fill('分頁一的標題'); await p1.getByRole('button', { name: '儲存設定' }).click(); await p1.waitForTimeout(800)
  await openSettings(p2); await p2.getByLabel('清單名稱').first().fill('分頁二的標題'); await p2.getByRole('button', { name: '儲存設定' }).click(); await p2.waitForTimeout(1000)
  L('S5.list-conflict', { p2msg: (await p2.locator('.c-err, [role=alert]').allInnerTexts()).join('|'), dbTitle: sql(`select title from wishlists where id='${T.id}'`) })
  await p2.screenshot({ path: OUT + '/shots/s5__conflict.png' })
  // 品項編輯衝突
  for (const p of [p1, p2]) await p.goto(`/lists/${T.id}/edit`, { waitUntil: 'networkidle' })
  await p1.getByRole('button', { name: '編輯', exact: true }).first().click(); await p2.getByRole('button', { name: '編輯', exact: true }).first().click()
  await p1.getByLabel('名稱（必填）').fill('品項-分頁一'); await p1.getByRole('button', { name: '儲存品項' }).click(); await p1.waitForTimeout(1000)
  await p2.getByLabel('名稱（必填）').fill('品項-分頁二'); await p2.getByRole('button', { name: '儲存品項' }).click(); await p2.waitForTimeout(1000)
  L('S5.item-conflict', { p2msg: (await p2.locator('.c-err, [role=alert]').allInnerTexts()).join('|'), dbItem: sql(`select title from wishlist_items where id='${T.items[0].id}'`) })
  await ctx.close()
})

// ---- S6 驚喜模式 ----
await step('S6', async () => {
  const U = await registerUser({ tag: 's6' }); const T = await createList(U, { title: 'S6 驚喜', surprise: true, showNames: true, items: [{ title: '禮物', qty: 3 }] })
  const g = await guestClaim(T.items[0].id, 1, '祕密客人', { note: '祕密留言' })
  const out = {}
  const pub = async () => (await (await (await guestApi()).get(`public/wishlists/${T.slug}`)).json())
  const dash = async () => (await (await U.api.get(`wishlists/${T.id}/dashboard`)).json())
  let p = await pub(), d = await dash(); out.before = { locked: d.surprise_locked, dashClaims: d.claims, dashItemClaimed: d.items[0].qty_claimed, pubClaimed: p.items[0].qty_claimed, pubClaimers: p.items[0].claimers ?? 'absent', claimersVisible: p.claimers_visible }
  
  const me = await (await U.api.get('me/export')); out.meExportStatus = me.status(); const txt = await me.text(); out.meExportLeaksClaimerWhileLocked = txt.includes('祕密客人') || txt.includes('祕密留言')
  const ctx = await newCtx(browser, { cookies: await sessionCookies(U) }); const page = await ctx.newPage()
  await page.goto(`/lists/${T.id}/progress`, { waitUntil: 'networkidle' }); out.progressLocked = (await page.innerText('main')).slice(0, 160).replace(/\n/g, '|')
  await page.screenshot({ path: OUT + '/shots/s6__progress-locked.png' })
  // 驚喜期間：擁有者改數量/刪除
  const del = await U.api.delete(`items/${T.items[0].id}`); out.deleteWhileLocked = del.status()
  const dn = await U.api.patch(`items/${T.items[0].id}`, { data: { qty_needed: 1 } }); out.shrinkWhileLocked = dn.status()
  // 擁有者以 API 取消他人認領（鎖定中應 403）
  const oc = await U.api.delete(`claims/${g.claimId}`); out.ownerCancelWhileLocked = oc.status()
  // 把 event_date 改成昨天（模擬活動日已過）
  sql(`update wishlists set event_date = (now() at time zone 'Asia/Taipei')::date - 1 where id='${T.id}'`)
  p = await pub(); d = await dash(); out.after = { locked: d.surprise_locked, dashClaims: (d.claims ?? []).map(c => c.claimer_name + ':' + c.note), pubClaimers: p.items[0].claimers ?? 'absent' }
  await page.reload({ waitUntil: 'networkidle' }); out.progressUnlocked = (await page.innerText('main')).slice(0, 200).replace(/\n/g, '|')
  await page.screenshot({ path: OUT + '/shots/s6__progress-unlocked.png' })
  // 剛好今天 00:00 邊界：event_date=今天（台北）
  sql(`update wishlists set event_date = (now() at time zone 'Asia/Taipei')::date where id='${T.id}'`); out.todayLocked = (await dash()).surprise_locked
  // 活動日前一天時，建立者開 /s/ 自己的頁
  sql(`update wishlists set event_date = (now() at time zone 'Asia/Taipei')::date + 1 where id='${T.id}'`); out.tomorrowLocked = (await dash()).surprise_locked
  // 鎖定中把 event_date 改成過去（關閉驚喜以外的繞過？）
  sql(`update wishlists set event_date = (now() at time zone 'Asia/Taipei')::date + 5 where id='${T.id}'`)
  const cur = await (await U.api.get(`wishlists/${T.id}`)).json()
  const bypass = await U.api.patch(`wishlists/${T.id}`, { data: { event_date: new Date(Date.now() - 864e5).toISOString().slice(0, 10), expected_updated_at: cur.wishlist.updated_at } }); out.setPastDateWhileLocked = bypass.status() + ' ' + (await bypass.text()).slice(0, 100)
  const mode = await U.api.patch(`wishlists/${T.id}`, { data: { surprise_mode: false } }); out.disableWhileLocked = mode.status()
  const ev = await U.api.patch(`wishlists/${T.id}`, { data: { event_date: null } }); out.clearDateWhileLocked = ev.status() + ' ' + (await ev.text()).slice(0, 100)
  L('S6', out); await ctx.close()
})

// ---- S8 訪客清除 storage / cookie；恢復流程；失效 token ----
await step('S8', async () => {
  sql('delete from rate_limits')
  const U = await registerUser({ tag: 's8' }); const T = await createList(U, { title: 'S8 清單', items: [{ title: '甲', qty: 3 }, { title: '乙', qty: 3 }] })
  const email = `s8-${uid()}@example.com`
  const ctx = await newCtx(browser); const page = await ctx.newPage()
  await page.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' }); await openCard(page, '甲'); const d = page.getByRole('dialog')
  await d.getByLabel('你的暱稱（必填）').fill('搬家哥'); await d.getByLabel(/^Email/).fill(email); await page.getByRole('button', { name: '確認認領' }).click(); await page.getByRole('heading', { name: '✓ 認領成功！' }).waitFor()
  const doneText = await page.locator('.g-sheet').innerText()
  await page.getByRole('button', { name: '回到清單繼續看' }).click()
  const mail = await waitForMail(email, { subject: /認領已確認/ }); const link = mail.text.match(/https?:\/\/\S+\/me\/claims#r=\S+/)?.[0]
  const out = { confirmMail: !!link, doneBanner: doneText.slice(0, 160).replace(/\n/g, '|') }
  // 清除 localStorage + cookie
  await ctx.clearCookies(); await page.evaluate(() => { localStorage.clear(); sessionStorage.clear() }); await page.reload({ waitUntil: 'networkidle' })
  out.afterClear_mineShown = await page.getByText('你已認領').isVisible().catch(() => false)
  out.afterClear_buttonOnClaimedItem = await page.locator('li.g-card', { hasText: '甲' }).locator('button').first().innerText()
  // 可再次認領同一品項（新 guest）
  await openCard(page, '甲'); await page.getByRole('dialog').getByLabel('你的暱稱（必填）').fill('搬家哥二號'); await page.getByRole('button', { name: '確認認領' }).click(); await page.waitForTimeout(1200)
  out.duplicateClaimAfterClear = await page.getByRole('heading', { name: '✓ 認領成功！' }).isVisible(); out.claimsOnItemA = sql(`select count(*) from claims where item_id='${T.items[0].id}' and status='reserved'`)
  // 恢復連結
  const ctx2 = await newCtx(browser); const p2 = await ctx2.newPage()
  await p2.goto(link.replace(/^https?:\/\/[^/]+/, WEB), { waitUntil: 'networkidle' }); await p2.waitForTimeout(800)
  out.recover = { url: p2.url().replace(WEB, ''), text: (await p2.innerText('main')).slice(0, 160).replace(/\n/g, '|') }
  await p2.goto(link.replace(/^https?:\/\/[^/]+/, WEB), { waitUntil: 'networkidle' }); await p2.waitForTimeout(500)
  out.recoverReuse = (await p2.innerText('main')).slice(0, 100).replace(/\n/g, '|')
  // 失效 token（來源：guest 自行刪除）
  const g = await guestClaim(T.items[1].id, 1, '會失效的人'); await g.api.delete('guest/me', { headers: { 'X-Guest-Token': g.token } })
  const ctx3 = await newCtx(browser); const p3 = await ctx3.newPage(); await p3.addInitScript(t => localStorage.setItem('ws_guest_token', t), g.token)
  await p3.goto(`/s/${T.slug}`, { waitUntil: 'networkidle' }); await openCard(p3, '乙'); const d3 = p3.getByRole('dialog')
  out.staleToken_nameFieldShown = await d3.getByLabel('你的暱稱（必填）').isVisible().catch(() => false)
  await d3.getByRole('button', { name: '確認認領' }).click(); await p3.waitForTimeout(1000)
  out.staleToken_msg = await d3.locator('.g-banner.err').innerText().catch(() => '(none)'); await p3.screenshot({ path: OUT + '/shots/s8__stale-token.png' })
  L('S8', out); await ctx.close(); await ctx2.close(); await ctx3.close()
})
await browser.close()
