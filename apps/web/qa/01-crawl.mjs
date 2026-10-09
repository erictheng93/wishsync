// 全站爬行：角色 × 視窗 × 配色。輸出 out/crawl.jsonl（稽核）、out/shots/*.png、out/crawl-issues.jsonl（console/網路）
import { chromium, newCtx, watch, AUDIT_FN, focusAudit, VIEWPORTS, OUT, WEB, API, log, sleep } from './lib.mjs'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
const seed = JSON.parse(readFileSync(OUT + '/seed.json'))
mkdirSync(OUT + '/shots', { recursive: true })
writeFileSync(OUT + '/crawl.jsonl', ''); writeFileSync(OUT + '/crawl-issues.jsonl', '')
const only = process.env.ONLY_VP
const vps = VIEWPORTS.filter(v => !only || v.name === only)

const open = (p, name) => async page => { await page.locator('li.g-card', { hasText: name }).first().getByRole('button').first().click(); await page.waitForTimeout(250) }
const scenes = {
  anon: [
    ['home', '/'], ['login', '/login'], ['register', '/register'], ['forgot', '/forgot-password'], ['unsub', '/unsubscribed'],
    ['terms', '/terms'], ['privacy', '/privacy'], ['offline', '/offline'], ['404', '/no-such-page'],
    ['s-active', `/s/${seed.active.slug}`], ['s-closed', `/s/${seed.closed.slug}`], ['s-hidden-410', `/s/${seed.hidden.slug}`], ['s-draft-404', `/s/${seed.draft.slug}`], ['s-bad-404', '/s/xxxxxxxxxx'],
    ['s-surprise', `/s/${seed.surprise.slug}`],
    ['s-claimsheet', `/s/${seed.active.slug}`, async page => { await open(0, '奶瓶組')(page) }],
    ['s-claimsheet-long', `/s/${seed.active.slug}`, async page => { await open(0, '很長很長')(page) }],
    ['s-reportsheet', `/s/${seed.active.slug}`, async page => { await page.getByRole('button', { name: '檢舉此清單' }).click(); await page.waitForTimeout(250) }],
    ['me-claims-anon', '/me/claims'],
    ['dashboard-redirect', '/dashboard'],
  ],
  guest: [
    ['s-active-mine', `/s/${seed.active.slug}`],
    ['s-claim-edit', `/s/${seed.active.slug}`, async page => { await page.locator('li.g-card', { hasText: '奶瓶組' }).getByRole('button', { name: '修改我的認領' }).click(); await page.waitForTimeout(250) }],
    ['me-claims', '/me/claims'],
  ],
  creator: [
    ['dashboard', '/dashboard'], ['new', '/lists/new'], ['settings', '/settings'],
    ['edit-active', `/lists/${seed.active.id}/edit`],
    ['edit-draft', `/lists/${seed.draft.id}/edit`],
    ['edit-closed', `/lists/${seed.closed.id}/edit`],
    ['edit-surprise', `/lists/${seed.surprise.id}/edit`],
    ['edit-itemsheet', `/lists/${seed.active.id}/edit`, async page => { await page.getByRole('button', { name: '編輯', exact: true }).first().click(); await page.waitForTimeout(300) }],
    ['edit-itemsheet-new', `/lists/${seed.active.id}/edit`, async page => { await page.getByRole('button', { name: '＋ 新增品項' }).click(); await page.waitForTimeout(300) }],
    ['edit-delconfirm', `/lists/${seed.active.id}/edit`, async page => { await page.getByRole('button', { name: '刪除' }).first().click(); await page.waitForTimeout(300) }],
    ['edit-share', `/lists/${seed.active.id}/edit`, async page => { await page.getByRole('button', { name: '分享' }).click(); await page.waitForTimeout(300) }],
    ['progress-active', `/lists/${seed.active.id}/progress`],
    ['progress-surprise', `/lists/${seed.surprise.id}/progress`],
    ['dash-archive-confirm', '/dashboard', async page => { await page.getByRole('button', { name: '封存' }).first().click(); await page.waitForTimeout(300) }],
    ['settings-delete-confirm', '/settings', async page => { await page.getByRole('button', { name: '刪除我的帳號' }).click(); await page.waitForTimeout(300) }],
    ['s-own-surprise', `/s/${seed.surprise.slug}`],
    ['s-own-active', `/s/${seed.active.slug}`],
    ['s-other-claimsheet-loggedin', `/s/${seed.other.slug}`, async page => { await page.locator('li.g-card').first().getByRole('button').first().click(); await page.waitForTimeout(300) }],
    ['edit-other-forbidden', `/lists/${seed.other.id}/edit`],
    ['login-when-authed', '/login'],
  ],
  staff: [
    ['admin-reports', '/admin'],
    ['admin-lists', '/admin', async page => { await page.getByRole('button', { name: '清單搜尋' }).click(); await page.waitForTimeout(500) }],
    ['admin-users', '/admin', async page => { await page.getByRole('button', { name: '使用者' }).click(); await page.waitForTimeout(500) }],
    ['admin-flags', '/admin', async page => { await page.getByRole('button', { name: '系統旗標' }).click(); await page.waitForTimeout(500) }],
    ['admin-modsheet', '/admin', async page => { await page.getByRole('button', { name: '下架' }).first().click(); await page.waitForTimeout(400) }],
    ['dashboard-staff', '/dashboard'],
  ],
  nonstaff_admin: [['admin-as-creator', '/admin']],
}

const browser = await chromium.launch()
for (const [role, list] of Object.entries(scenes)) {
  for (const vp of vps) for (const scheme of ['light', 'dark']) {
    const cookies = role === 'creator' ? seed.A.cookies : role === 'staff' ? seed.S.cookies : role === 'nonstaff_admin' ? seed.A.cookies : undefined
    const ctx = await newCtx(browser, { viewport: vp, scheme, cookies })
    if (role === 'guest') {
      await ctx.addCookies([{ name: 'ws_guest', value: seed.g1.token, url: API }])
      await ctx.addInitScript(t => { try { localStorage.setItem('ws_guest_token', t) } catch {} }, seed.g1.token)
    }
    for (const [name, path, act] of list) {
      const tag = `${role}/${name}/${vp.name}/${scheme}`
      const page = await ctx.newPage()
      const sink = []; watch(page, tag, sink)
      try {
        await page.goto(path, { waitUntil: 'networkidle', timeout: 20000 })
        await page.waitForTimeout(500)
        if (act) await act(page)
        const audit = await page.evaluate(AUDIT_FN)
        const focus = (vp.name === '1440' || vp.name === '390') && scheme === 'light' ? await focusAudit(page, 25) : null
        const noFocus = focus?.filter(f => !f.outline && !f.shadow && f.visible).map(f => f.el)
        await page.screenshot({ path: `${OUT}/shots/${role}__${name}__${vp.name}__${scheme}.png`, fullPage: false })
        log('crawl.jsonl', { tag, url: page.url().replace(WEB, ''), audit, focusCount: focus?.length, noFocus })
      } catch (e) { log('crawl.jsonl', { tag, error: String(e).slice(0, 300) }) }
      for (const s of sink) log('crawl-issues.jsonl', s)
      await page.close()
    }
    await ctx.close()
  }
}
await browser.close()
console.log('done')
