import { describe, expect, it } from 'vitest'
import { buildOrderPatch, contributionErr, deriveDisplayStatus, displayStatusLabel, fmtTime, fromTaipeiInput, grantError, label, LEDGER_TYPE, mergeItemEvent, normWalletRow, orderActions, quickAmounts, toTaipeiInput } from '../app/utils/crowdfund'

describe('display_status', () => {
  it('六種狀態都有中文', () => {
    expect(['open', 'funded', 'ordered', 'shipped', 'delivered', 'expired'].map(displayStatusLabel)).toEqual(['募集中', '已達標', '已下單', '運送中', '已送達', '已截止'])
    expect(displayStatusLabel(null)).toBe('')
  })
  it('優先用後端 display_status，沒有才由 funding_status + 採購單推導', () => {
    expect(deriveDisplayStatus({ display_status: 'shipped', funding_status: 'funded' })).toBe('shipped')
    expect(deriveDisplayStatus({ funding_status: 'open' })).toBe('open')
    expect(deriveDisplayStatus({ funding_status: 'funded' })).toBe('funded')
    expect(deriveDisplayStatus({ funding_status: 'funded' }, { status: 'placed' })).toBe('ordered')
    expect(deriveDisplayStatus({ funding_status: 'funded' }, { status: 'shipped' })).toBe('shipped')
    expect(deriveDisplayStatus({ funding_status: 'fulfilled' })).toBe('delivered')
    expect(deriveDisplayStatus({ funding_status: null })).toBeNull()
  })
  it('帳本類型中文，未知值原樣', () => {
    expect(label(LEDGER_TYPE, 'refund')).toBe('差額退回')
    expect(label(LEDGER_TYPE, 'weird')).toBe('weird')
  })
})

describe('contributionErr', () => {
  it('點數不足顯示餘額並導向錢包', () => {
    expect(contributionErr('INSUFFICIENT_POINTS', { balance: 1200 })).toEqual({ msg: '點數不足（目前餘額 1,200 點）', wallet: true })
  })
  it('超過目標 → 自動改成 remaining_points', () => {
    expect(contributionErr('CROWDFUND_TARGET_EXCEEDED', { remaining_points: 300 }).points).toBe(300)
    expect(contributionErr('CROWDFUND_TARGET_EXCEEDED', { remaining_points: 0 }).points).toBeUndefined()
  })
  it.each([['ITEM_FUNDED', '募集完成'], ['FUNDING_EXPIRED', '期限已過'], ['WALLET_FROZEN', '凍結']])('%s', (c, part) => expect(contributionErr(c).msg).toContain(part))
  it('未知 code 用 fallback', () => expect(contributionErr('X', {}, '壞了').msg).toBe('壞了'))
  it('快選金額只列小於剩餘的', () => {
    expect(quickAmounts(600)).toEqual([100, 500])
    expect(quickAmounts(50)).toEqual([])
  })
})

describe('時間', () => {
  it('以台北時間顯示', () => expect(fmtTime('2026-10-08T05:10:00Z')).toBe('2026/10/08 13:10'))
  it('跨日換算', () => expect(fmtTime('2026-12-01T15:59:00Z')).toBe('2026/12/01 23:59'))
  it('datetime-local 與 ISO 互轉（台北）', () => {
    expect(toTaipeiInput('2026-12-01T15:59:00Z')).toBe('2026-12-01T23:59')
    expect(fromTaipeiInput('2026-12-01T23:59')).toBe('2026-12-01T23:59:00+08:00')
    expect(new Date(fromTaipeiInput('2026-12-01T23:59')!).toISOString()).toBe('2026-12-01T15:59:00.000Z')
    expect(fromTaipeiInput('')).toBeNull()
  })
})

describe('mergeItemEvent', () => {
  const crowd = { id: 'a', funding_mode: 'crowdfund', target_points: 1000, pledged_points: 0, contributors: [{ display_name: 'x', points: 1 }] }
  it('眾籌：重算 remaining / progress，保留其他欄位', () => {
    const m = mergeItemEvent(crowd, { item_id: 'a', pledged_points: 999, target_points: 1000, funding_status: 'open', qty_claimed: 0, qty_needed: 1 })
    expect(m).toMatchObject({ id: 'a', remaining_points: 1, progress_percent: 99, contributors: crowd.contributors })
  })
  it('眾籌：達標 100%', () => expect(mergeItemEvent(crowd, { pledged_points: 1000, display_status: 'funded' })).toMatchObject({ progress_percent: 100, remaining_points: 0, display_status: 'funded' }))
  it('眾籌：pledged 為 null（驚喜鎖定）不重算', () => expect(mergeItemEvent(crowd, { pledged_points: null }).progress_percent).toBeUndefined())
  it('數量品項沿用原算法', () => expect(mergeItemEvent({ id: 'q', funding_mode: 'quantity' }, { qty_needed: 4, qty_claimed: 1 }).progress_percent).toBe(25))
})

describe('採購單', () => {
  const f = { merchant_order_id: '', amount: '', tracking_no: '', failure_reason: '' }
  it('合法動作依狀態', () => {
    expect(orderActions('pending')).toEqual(['placed', 'failed', 'cancelled'])
    expect(orderActions('placed')).toEqual(['shipped', 'failed', 'cancelled'])
    expect(orderActions('shipped')).toEqual(['delivered'])
    expect(orderActions('delivered')).toEqual([]); expect(orderActions('failed')).toEqual([])
  })
  it('placed 必填訂單編號與金額，且 <= 目標', () => {
    expect(buildOrderPatch('placed', f, 1000).error).toBeTruthy()
    expect(buildOrderPatch('placed', { ...f, merchant_order_id: 'S1', amount: '1001' }, 1000).error).toContain('不可超過')
    expect(buildOrderPatch('placed', { ...f, merchant_order_id: ' S1 ', amount: '900' }, 1000).body).toEqual({ status: 'placed', merchant_order_id: 'S1', amount: 900 })
  })
  it('failed / cancelled 必填原因；shipped 單號選填', () => {
    expect(buildOrderPatch('failed', f, 1).error).toBe('請填原因')
    expect(buildOrderPatch('cancelled', { ...f, failure_reason: '缺貨' }, 1).body).toEqual({ status: 'cancelled', failure_reason: '缺貨' })
    expect(buildOrderPatch('shipped', f, 1).body).toEqual({ status: 'shipped' })
    expect(buildOrderPatch('shipped', { ...f, tracking_no: 'TW1' }, 1).body).toEqual({ status: 'shipped', tracking_no: 'TW1' })
    expect(buildOrderPatch('delivered', f, 1).body).toEqual({ status: 'delivered' })
  })
})

describe('後台錢包', () => {
  it('發 / 扣點驗證', () => {
    expect(grantError('', 'x')).toBeTruthy(); expect(grantError(0, 'x')).toBeTruthy(); expect(grantError('1.5', 'x')).toBeTruthy()
    expect(grantError(1_000_001, 'x')).toContain('上限'); expect(grantError(100, ' ')).toBe('請填原因')
    expect(grantError(-100, '補償')).toBe(''); expect(grantError('500', '活動')).toBe('')
  })
  it('wallet 列容忍巢狀與扁平', () => {
    const want = { user_id: 'u', display_name: 'A', email: 'a@x', wallet_id: 'w', balance: 5, status: 'frozen' }
    expect(normWalletRow({ user: { id: 'u', display_name: 'A', email: 'a@x' }, wallet: { id: 'w', balance: 5, status: 'frozen' } })).toEqual(want)
    expect(normWalletRow({ user_id: 'u', display_name: 'A', email: 'a@x', wallet_id: 'w', balance: 5, status: 'frozen' })).toEqual(want)
    expect(normWalletRow({ user: { id: 'u' }, wallet: null })).toMatchObject({ wallet_id: null, balance: 0, status: 'active' })
  })
})
