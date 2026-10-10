// SSE 即時更新；連續失敗 2 次即開輪詢（onEvent('poll')），連上後停止輪詢
export function useWishlistEvents(slug: Ref<string | null | undefined>, onEvent: (type: string) => void, pollMs = 15000) {
  const { base } = useApi()
  const status = ref<'idle' | 'live' | 'polling'>('idle')
  let es: EventSource | null = null, timer: any = null, errs = 0
  const stopPoll = () => { if (timer) { clearInterval(timer); timer = null } }
  const startPoll = () => { if (!timer) { status.value = 'polling'; timer = setInterval(() => onEvent('poll'), pollMs) } }
  function close() { es?.close(); es = null }
  function open() {
    close()
    if (!slug.value) return
    if (typeof EventSource === 'undefined') return startPoll()
    es = new EventSource(`${base}/public/wishlists/${slug.value}/events${getListAccess(slug.value) ? `?access=${encodeURIComponent(getListAccess(slug.value)!)}` : ''}`, { withCredentials: true })
    es.onopen = () => { errs = 0; stopPoll(); status.value = 'live' }
    for (const t of ['item.updated', 'wishlist.updated']) es.addEventListener(t, () => onEvent(t))
    es.onerror = () => {
      errs++
      if (errs >= 2) startPoll()
      if (es?.readyState === 2) { startPoll() } // 伺服器拒絕（404/410）：瀏覽器不再重連，改純輪詢
    }
  }
  onMounted(() => { watch(slug, open, { immediate: true }) })
  onBeforeUnmount(() => { close(); stopPoll() })
  return { status }
}
