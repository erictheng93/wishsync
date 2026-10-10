// P2-A 點數眾籌：純函式（文案對應、時間格式、錯誤歸因、表單轉換），可單元測試
export const DISPLAY_STATUS: Record<string, string> = { open: '募集中', funded: '已達標', ordered: '已下單', shipped: '運送中', delivered: '已送達', expired: '已截止' }
export const displayStatusLabel = (s?: string | null) => (s && DISPLAY_STATUS[s]) || ''

/** 優先用後端的 display_status；沒有時（例如 dashboard 只回 funding_status）用 funding_status + 採購單狀態推導 */
export function deriveDisplayStatus(item: { display_status?: string | null, funding_status?: string | null }, order?: { status?: string } | null): string | null {
  if (item.display_status) return item.display_status
  const f = item.funding_status
  if (f === 'open' || f === 'expired') return f
  if (f === 'fulfilled') return 'delivered'
  if (f === 'funded') return ({ placed: 'ordered', shipped: 'shipped', delivered: 'delivered' } as Record<string, string>)[order?.status ?? ''] ?? 'funded'
  return null
}

export const CONTRIBUTION_STATUS: Record<string, string> = { pledged: '已認捐', captured: '已達標扣用', released: '已退回', reallocated: '已轉投' }
export const LEDGER_TYPE: Record<string, string> = { grant: '平台發放', pledge: '認捐', release: '退回', refund: '差額退回', adjustment: '調整' }
export const ORDER_STATUS: Record<string, string> = { pending: '待下單', placed: '已下單', shipped: '運送中', delivered: '已送達', failed: '失敗', cancelled: '已取消' }
export const label = (m: Record<string, string>, k?: string | null) => (k && m[k]) || k || ''

const tf = new Intl.DateTimeFormat('zh-TW', { timeZone: 'Asia/Taipei', year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
/** zh-TW、Asia/Taipei，例如 2026/10/08 13:10 */
export const fmtTime = (iso?: string | null) => (iso ? tf.format(new Date(iso)).replace(/\s/g, ' ') : '') // ICU 可能用不換行空白分隔日期與時間
export const fmtPts = (n?: number | null) => (n == null ? '–' : n.toLocaleString('en-US'))

/** 公開頁 SSE item.updated 合併進卡片。眾籌品項用 pledged/target 重算進度，數量品項維持原算法 */
export function mergeItemEvent(i: any, d: any) {
  const m = { ...i, ...d, id: i.id }
  if (i.funding_mode === 'crowdfund') {
    const t = m.target_points, p = m.pledged_points
    if (t > 0 && p != null) { m.remaining_points = Math.max(0, t - p); m.progress_percent = Math.min(100, Math.floor(p / t * 100)) }
  } else m.progress_percent = Math.min(100, Math.round(m.qty_claimed / m.qty_needed * 100))
  return m
}

/** 贊助失敗 → 文案；points 表示要把輸入框改成這個值 */
export function contributionErr(code: string, body: any = {}, fallback = '贊助失敗，請稍後再試'): { msg: string, points?: number, wallet?: boolean } {
  switch (code) {
    case 'INSUFFICIENT_POINTS': return { msg: `點數不足（目前餘額 ${fmtPts(body.balance ?? 0)} 點）`, wallet: true }
    case 'CROWDFUND_TARGET_EXCEEDED': {
      const r = Number(body.remaining_points ?? 0)
      return r > 0 ? { msg: `只差 ${fmtPts(r)} 點就達標了，已幫你調整金額，確認後再送出一次`, points: r } : { msg: '這個品項剛剛已經募集完成了，謝謝你的心意' }
    }
    case 'ITEM_FUNDED': return { msg: '這個品項剛剛已經募集完成了，謝謝你的心意' }
    case 'FUNDING_EXPIRED': return { msg: '這個品項的募集期限已過，無法再贊助' }
    case 'WALLET_FROZEN': return { msg: '你的點數錢包已被凍結，暫時無法贊助，請聯絡客服' }
    case 'WISHLIST_CLOSED': return { msg: '這份清單已結束，無法再贊助' }
    case 'WISHLIST_REMOVED': return { msg: '這份清單已被下架' }
    case 'FUNDING_MODE_MISMATCH': return { msg: '這個品項不是點數眾籌，請改用「我要送」' }
    case 'IDEMPOTENCY_CONFLICT': return { msg: '請求重複，請關閉後重新操作' }
    case 'RATE_LIMITED': return { msg: '操作太頻繁，請稍後再試' }
    case 'NETWORK': return { msg: '網路不穩，再試一次' }
    default: return { msg: fallback }
  }
}
/** 快選金額：只列小於剩餘的幾檔，「剩餘全額」由畫面另外加 */
export const quickAmounts = (remaining: number) => [100, 500, 1000].filter(n => n < remaining)

// datetime-local 以台北時間輸入 / 顯示（與後端「活動日 23:59 台北時間」一致）
export const toTaipeiInput = (iso?: string | null) => (iso ? new Date(Date.parse(iso) + 8 * 36e5).toISOString().slice(0, 16) : '')
export const fromTaipeiInput = (v?: string | null) => (v ? `${v}:00+08:00` : null)

// --- 後台採購單 ---
export const ORDER_NEXT: Record<string, string[]> = { pending: ['placed', 'failed', 'cancelled'], placed: ['shipped', 'failed', 'cancelled'], shipped: ['delivered'] }
export const orderActions = (status: string) => ORDER_NEXT[status] ?? []
export interface OrderForm { merchant_order_id: string, amount: string | number, tracking_no: string, failure_reason: string }
/** 依目標狀態組 PATCH body 並做前端驗證（後端仍會再驗） */
export function buildOrderPatch(to: string, f: OrderForm, target: number | null): { body?: any, error?: string } {
  if (to === 'placed') {
    const amount = Number(f.amount)
    if (!f.merchant_order_id.trim()) return { error: '請填訂單編號' }
    if (f.amount === '' || !Number.isInteger(amount) || amount <= 0) return { error: '請填實際金額（正整數）' }
    if (target != null && amount > target) return { error: `實際金額不可超過目標 ${fmtPts(target)}，超過請改標為失敗` }
    return { body: { status: to, merchant_order_id: f.merchant_order_id.trim(), amount } }
  }
  if (to === 'shipped') return { body: { status: to, ...(f.tracking_no.trim() ? { tracking_no: f.tracking_no.trim() } : {}) } }
  if (to === 'failed' || to === 'cancelled') {
    if (!f.failure_reason.trim()) return { error: '請填原因' }
    return { body: { status: to, failure_reason: f.failure_reason.trim() } }
  }
  return { body: { status: to } }
}

// --- 後台錢包 ---
export function grantError(points: string | number, reason: string): string {
  const n = Number(points)
  if (points === '' || !Number.isInteger(n) || n === 0) return '點數需為非 0 的整數（扣點用負數）'
  if (Math.abs(n) > 1_000_000) return '單次上限 1,000,000 點'
  if (!reason.trim()) return '請填原因'
  return ''
}
/** /admin/wallets 列：同時容忍巢狀（user + wallet）與扁平兩種 shape */
export function normWalletRow(r: any) {
  return {
    user_id: r.user?.id ?? r.user_id ?? null, display_name: r.user?.display_name ?? r.display_name ?? '', email: r.user?.email ?? r.email ?? '',
    wallet_id: r.wallet?.id ?? r.wallet_id ?? null, balance: r.wallet?.balance ?? r.balance ?? 0, status: r.wallet?.status ?? r.status ?? 'active',
  }
}
