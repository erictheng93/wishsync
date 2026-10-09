// SSE 即時更新 + 輪詢備援（docs 03 §14 S3、docs 04 §7.3）。
// SSE 連續失敗（CLOSED、累計 3 次錯誤、或 30 秒內未連上）或瀏覽器不支援 → 每 15 秒 refresh()；
// 分頁隱藏時暫停，回到前景立即刷新一次；SSE 重新連上後停止輪詢。
// 純函式、不依賴 Vue / Nuxt，方便單元測試。
export type LiveMode = 'connecting' | 'live' | 'polling'
export interface LiveOpts {
  url: string
  refresh: () => unknown
  onEvent: (type: string, data: any) => void
  onMode: (m: LiveMode) => void
  eventTypes?: string[]
  pollMs?: number, failLimit?: number, connectTimeoutMs?: number
}

export function createLiveRefresh(o: LiveOpts) {
  const { pollMs = 15000, failLimit = 3, connectTimeoutMs = 30000, eventTypes = ['item.updated', 'wishlist.updated'] } = o
  let es: EventSource | undefined, poll: ReturnType<typeof setInterval> | undefined, connectTimer: ReturnType<typeof setTimeout> | undefined
  let fails = 0, mode: LiveMode = 'connecting'
  const set = (m: LiveMode) => { if (m !== mode) { mode = m; o.onMode(m) } }
  const hidden = () => typeof document !== 'undefined' && document.visibilityState === 'hidden'
  const onVisible = () => { if (mode === 'polling' && !hidden()) o.refresh() }

  function startPoll() {
    clearTimeout(connectTimer)
    set('polling')
    poll ??= setInterval(() => { if (!hidden()) o.refresh() }, pollMs)
  }
  function stopPoll() { clearInterval(poll); poll = undefined }

  function start() {
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisible)
    if (typeof EventSource === 'undefined') return startPoll()
    es = new EventSource(o.url)
    connectTimer = setTimeout(() => { if (mode !== 'live') startPoll() }, connectTimeoutMs)
    es.onopen = () => { fails = 0; clearTimeout(connectTimer); stopPoll(); set('live') }
    es.onerror = () => {
      fails++
      const closed = es?.readyState === 2 // 被伺服器拒絕（404/410）：瀏覽器不再重連，重抓一次讓頁面顯示下架 / 找不到
      if (mode === 'live') set('connecting')
      if (closed || fails >= failLimit) { startPoll(); if (closed) o.refresh() }
    }
    for (const t of eventTypes) es.addEventListener(t, (ev: any) => { try { o.onEvent(t, JSON.parse(ev.data)) } catch { o.onEvent(t, null) } })
  }
  function stop() {
    es?.close(); es = undefined; stopPoll(); clearTimeout(connectTimer)
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisible)
  }
  return { start, stop }
}

export const liveLabel = (m: LiveMode) => m === 'live' ? '即時更新中' : m === 'polling' ? '定時更新' : '連線中…'
