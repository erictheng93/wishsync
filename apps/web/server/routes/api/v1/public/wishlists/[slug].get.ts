// S1 預覽專用：僅在 NUXT_DEMO=1（見 wrangler.jsonc）時回傳固定資料，正式環境走 apiBase 指向的真實 API，這支不會被呼叫
export default defineEventHandler((event) => {
  if (!useRuntimeConfig(event).demo) throw createError({ statusCode: 404 })
  if (getRouterParam(event, 'slug') !== 'demo') throw createError({ statusCode: 404 })
  const raw = [
    // 點數眾籌品項（P2-A）：示範卡片；demo 沒有後端，點「用點數贊助」會因 /me 失敗而導到登入頁
    { id: 'i1', title: 'Combi 嬰兒推車', priority: 'high', qty_needed: 1, qty_claimed: 0, unit_price_amount: null, funding_mode: 'crowdfund',
      target_points: 9800, pledged_points: 6300, remaining_points: 3500, funding_status: 'open', display_status: 'open', funding_deadline: '2026-12-19T15:59:00Z',
      contributors: [{ display_name: '阿明', points: 3000 }, { display_name: '匿名朋友', points: 1300 }, { display_name: '小華', points: 2000 }] },
    { id: 'i2', title: 'NB 尿布 2 包', priority: 'medium', qty_needed: 2, qty_claimed: 1, unit_price_amount: 300 },
    { id: 'i3', title: '奶粉 1 號 800g', priority: 'low', qty_needed: 2, qty_claimed: 2, unit_price_amount: 800 },
  ]
  const items = raw.map(i => ({ funding_mode: 'quantity', brand: null, spec: null, image_url: null, product_url: null, ...i,
    qty_remaining: i.qty_needed - i.qty_claimed, is_fully_claimed: i.qty_claimed >= i.qty_needed, progress_percent: (i as any).target_points ? Math.floor((i as any).pledged_points / (i as any).target_points * 100) : Math.round(i.qty_claimed / i.qty_needed * 100) }))
  const claimed = raw.reduce((a, i) => a + i.qty_claimed, 0), needed = raw.reduce((a, i) => a + i.qty_needed, 0)
  return { id: 'w1', slug: 'demo', type: 'registry', status: 'active', title: '小愛的待產清單', description: '預產期 12 月，謝謝大家的心意',
    cover_image_url: null, event_date: '2026-12-20', owner: { id: 'u1', display_name: '小愛' }, surprise_mode: false, claimers_visible: false,
    completion: { item_count: items.length, fulfilled_count: items.filter(i => i.is_fully_claimed).length, completion_pct: Math.round(claimed / needed * 100) },
    items, updated_at: new Date().toISOString() }
})
