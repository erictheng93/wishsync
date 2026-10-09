import { chromium, newCtx, registerUser, createList, sessionCookies, log, OUT, sleep, pwRequest } from './lib.mjs'
import { writeFileSync, readFileSync } from 'node:fs'
import { execSync } from 'node:child_process'
writeFileSync(OUT + '/04.jsonl', '')
const CF = 'http://localhost:3015', ND = 'http://localhost:3014'
const L = (tag, o) => { console.log(tag, JSON.stringify(o).slice(0, 400)); log('04.jsonl', { tag, ...o }) }
const seed = JSON.parse(readFileSync(OUT + '/seed.json'))
const api = await pwRequest.newContext()
// 1 manifest + icons
for (const base of [CF, ND]) {
  const r = await api.get(base + '/manifest.webmanifest'); const m = await r.json()
  const res = { status: r.status(), ct: r.headers()['content-type'], cc: r.headers()['cache-control'], keys: Object.keys(m).join(','), start_url: m.start_url, scope: m.scope, display: m.display, icons: [] }
  for (const i of m.icons) {
    const ir = await api.get(base + i.src); const b = await ir.body()
    const w = b.readUInt32BE(16), h = b.readUInt32BE(20)
    res.icons.push(`${i.src} ${ir.status()} ${ir.headers()['content-type']} declared=${i.sizes} actual=${w}x${h} purpose=${i.purpose} cc=${ir.headers()['cache-control']}`)
  }
  for (const p of ['/apple-touch-icon.png', '/og-default.png', '/offline', '/sw.js']) { const x = await api.get(base + p); res[p] = `${x.status()} ${x.headers()['content-type']} cc=${x.headers()['cache-control']}` }
  L('manifest@' + base, res)
}
const browser = await chromium.launch()
// 2 SW on node-server (3014)
{
  const ctx = await newCtx(browser); const page = await ctx.newPage()
  await page.goto(ND + '/login', { waitUntil: 'networkidle' }); await sleep(3000)
  const st = await page.evaluate(async () => { const r = await navigator.serviceWorker.getRegistration(); return r ? { scope: r.scope, active: r.active?.state, installing: r.installing?.state, waiting: r.waiting?.state } : null })
  L('sw@node-server3014', { reg: st, caches: await page.evaluate(async () => Object.fromEntries(await Promise.all((await caches.keys()).map(async k => [k, (await (await caches.open(k)).keys()).map(r => new URL(r.url).pathname)])))) })
  await ctx.close()
}
// 3 SW on CF-like (3015)
const U = await registerUser({ tag: 'pwa' }); const T = await createList(U, { title: 'PWA 清單', items: [{ title: 'pwa品項', qty: 2 }] })
const ctx = await newCtx(browser, { cookies: await sessionCookies(U) }); const page = await ctx.newPage()
const reqs = []; page.on('console', m => { if (m.type() === 'error') reqs.push('console: ' + m.text().slice(0, 120)) })
await page.goto(CF + '/login', { waitUntil: 'networkidle' }); await sleep(2500)
const reg = await page.evaluate(async () => { const r = await navigator.serviceWorker.ready; return { scope: r.scope, active: r.active?.state, scriptURL: r.active?.scriptURL, updateViaCache: r.updateViaCache } })
L('sw@cf3015.register', reg)
await page.reload({ waitUntil: 'networkidle' }); L('sw.controlled-after-reload', { controller: await page.evaluate(() => !!navigator.serviceWorker.controller) })
// 逛一輪：分享頁、儀表板、設定、離線頁、API
for (const p of [`/s/${T.slug}`, '/dashboard', '/settings', `/lists/${T.id}/edit`, '/me/claims', '/terms', '/offline']) { await page.goto(CF + p, { waitUntil: 'networkidle' }).catch(() => {}); await sleep(600) }
const dump = async () => page.evaluate(async () => Object.fromEntries(await Promise.all((await caches.keys()).map(async k => [k, (await (await caches.open(k)).keys()).map(r => new URL(r.url).pathname + new URL(r.url).search)]))))
let cc = await dump(); const all = Object.values(cc).flat()
L('sw.cache-contents', { keys: Object.keys(cc), count: all.length, nonStatic: all.filter(p => !/^\/(_nuxt\/|icons\/|offline$)/.test(p)), sample: all.slice(0, 8), hasS: all.some(p => p.startsWith('/s/')), hasApi: all.some(p => p.includes('/api')), hasBuildsMeta: all.some(p => p.startsWith('/_nuxt/builds')) })
// SSR 分享頁：被 SW 攔截的導覽是否仍為最新（改標題後立刻重新整理）
await U.api.patch(`wishlists/${T.id}`, { data: { title: 'PWA 清單-已改標題', expected_updated_at: (await (await U.api.get(`wishlists/${T.id}`)).json()).wishlist.updated_at } })
await page.goto(CF + `/s/${T.slug}`, { waitUntil: 'networkidle' }); L('sw.share-fresh', { h1: await page.locator('h1').first().innerText() })
// 另一使用者（無 cookie）看到的 /dashboard 不會是前一個人的（導覽不被快取）
const ctxB = await newCtx(browser); const pb = await ctxB.newPage(); await pb.goto(CF + '/dashboard', { waitUntil: 'networkidle' }); await sleep(800); L('sw.cross-user-dashboard', { url: pb.url().replace(CF, ''), hasMyList: (await pb.content()).includes('PWA 清單') }); await ctxB.close()
// 4 更新流程：改 dist/sw.js 的 CACHE_VERSION
const swPath = new URL('./.app-cf/dist/sw.js', import.meta.url).pathname
const orig = readFileSync(swPath, 'utf8'); writeFileSync(swPath + '.bak', orig)
writeFileSync(swPath, orig.replace("CACHE_VERSION = 'v1'", "CACHE_VERSION = 'v2'"))
const upd = await page.evaluate(async () => { const r = await navigator.serviceWorker.ready; await r.update(); await new Promise(r => setTimeout(r, 1500)); const x = await navigator.serviceWorker.getRegistration(); return { active: x.active?.state, waiting: x.waiting?.state, installing: x.installing?.state } })
cc = await dump(); L('sw.update.after-update()', { ...upd, cacheKeys: Object.keys(cc) })
await page.close(); await sleep(300)
// 所有分頁關閉後，新版接手
const p2 = await ctx.newPage(); await p2.goto(CF + '/terms', { waitUntil: 'networkidle' }); await sleep(2500)
await p2.reload({ waitUntil: 'networkidle' }); await sleep(1000)
const cc2 = await p2.evaluate(async () => ({ keys: await caches.keys(), swUrl: (await navigator.serviceWorker.getRegistration())?.active?.state }))
L('sw.update.after-reopen', cc2)
// 5 離線：關閉 web server（wrangler）
const pid = Number(readFileSync(OUT + '/wrangler.pid', 'utf8')); 
try { execSync(`kill ${pid}`) } catch {}
try { execSync(`pkill -P ${pid}`) } catch {}
await sleep(3000)
const off = {}
for (const p of [`/s/${T.slug}`, '/dashboard', '/terms', '/offline']) {
  const pg = await ctx.newPage(); const t = await pg.goto(CF + p, { waitUntil: 'load', timeout: 15000 }).then(r => r?.status()).catch(e => 'ERR ' + String(e).slice(0, 60)); await sleep(500)
  off[p] = { status: t, text: (await pg.innerText('body').catch(() => '')).slice(0, 80).replace(/\n/g, '|') }
  if (p === '/s/' + T.slug) await pg.screenshot({ path: OUT + '/shots/pwa__offline-share.png' })
  await pg.close()
}
L('sw.offline-server-down', off)
// 離線後 /_nuxt 資產（已快取）仍可取得
const asset = await ctx.newPage(); await asset.goto('about:blank')
const nuxtAsset = cc2.keys.length ? await p2.evaluate(async () => { const ks = await caches.keys(); const c = await caches.open(ks[0]); const reqs = (await c.keys()).filter(r => r.url.includes('/_nuxt/')); if (!reqs.length) return 'none cached'; const r = await fetch(reqs[0].url).then(x => x.status).catch(e => 'ERR ' + e.message); return r }) : null
L('sw.offline-cached-asset', { nuxtAsset })
writeFileSync(swPath, orig)
await browser.close(); console.log('done')
