<script setup lang="ts">
import '~/assets/guest.css'
// 公開分享頁：必須 SSR，LINE / FB 爬蟲才讀得到 OG（docs 03 §5.6）
const route = useRoute()
const slug = route.params.slug as string
const { public: { apiBase } } = useRuntimeConfig()
const { api } = useGuest()

// 不用 useFetch 的 error 拋出：404 / 410 在頁內顯示，並設正確 HTTP 狀態碼（410 需 noindex、不洩漏標題）
const { data, error, refresh } = await useFetch<any>(`${apiBase}/api/v1/public/wishlists/${slug}`, { credentials: 'omit' })
const gone = computed(() => error.value?.statusCode === 410)
if (error.value && import.meta.server) setResponseStatus(useRequestEvent()!, gone.value ? 410 : (error.value.statusCode ?? 500))

if (data.value) {
  // 規格：docs 03 §5.6 meta 內容規則
  const left = () => (data.value?.items ?? []).filter((i: any) => !i.is_fully_claimed).length
  const origin = useRequestURL().origin
  useSeoMeta({
    title: () => data.value?.title,
    ogTitle: () => `幫${data.value?.owner?.display_name ?? '朋友'}挑禮物｜${data.value?.title}`,
    description: () => data.value?.description ?? undefined,
    ogDescription: () => `已完成 ${data.value?.completion?.completion_pct ?? 0}%，還有 ${left()} 項等你認領。不用註冊，點開就能選。`,
    // ponytail: 預設圖為靜態檔；動態 /og/{slug}.png（含清單名稱與完成度）之後再做
    ogImage: () => data.value?.cover_image_url ?? `${origin}/og-default.png`,
    ogImageWidth: 1200, ogImageHeight: 630,
    ogType: 'website', ogUrl: () => `${origin}/s/${slug}`, ogLocale: 'zh_TW', twitterCard: 'summary_large_image',
  })
} else useSeoMeta({ title: gone.value ? '此清單已被下架' : '找不到這份清單', robots: 'noindex' })

const items = ref<any[]>(data.value?.items ?? [])
watch(data, v => { if (v) items.value = v.items })
// 剩餘優先 → 已滿排最後（穩定排序）
const sorted = computed(() => [...items.value].sort((a, b) => Number(a.is_fully_claimed) - Number(b.is_fully_claimed)))
const pct = computed(() => data.value?.completion?.completion_pct ?? 0)
const closed = computed(() => data.value?.status === 'closed')

// --- 線上狀態 / LINE 引導 ---
const online = ref(true), live = ref(false), lineHint = ref(false)
// --- 我的認領（有 token 時 GET /guest/me）---
const myClaims = ref<Record<string, any>>({})
async function loadMine() {
  try { // 不先檢查 token：登入使用者靠 session cookie 認領，沒有 guest token
    const r = await api('/guest/me?limit=100')
    myClaims.value = Object.fromEntries(r.claims.filter((c: any) => c.wishlist.slug === slug && ['reserved', 'purchased', 'delivered'].includes(c.claim.status)).map((c: any) => [c.claim.item_id, c.claim]))
  } catch { /* 401：token 失效，當作沒有 */ }
}
// 建立者本人 + 驚喜模式 → 軟性遮蔽（GET /me 比對 owner.id）
const isOwner = ref(false)
const me = ref<any>(null) // 已登入使用者：認領以帳號名稱進行，不再問暱稱
const masked = computed(() => isOwner.value && !!data.value?.surprise_mode)

let es: EventSource | undefined
onMounted(async () => {
  online.value = navigator.onLine
  addEventListener('online', () => { online.value = true; refresh() }); addEventListener('offline', () => (online.value = false))
  lineHint.value = isLineBrowser() && !getPref('ws_hint_dismissed')
  loadMine()
  $fetch<any>(`${apiBase}/api/v1/me`, { credentials: 'include' }).then(m => { me.value = m; isOwner.value = m?.id === data.value?.owner?.id }).catch(() => {})
  if (!data.value) return
  es = new EventSource(`${apiBase}/api/v1/public/wishlists/${slug}/events`)
  es.onopen = () => (live.value = true)
  // EventSource 會自動重連；被伺服器拒絕（410/404）時 readyState=CLOSED 不再重連，改重抓一次讓頁面顯示下架 / 找不到
  es.onerror = () => { live.value = false; if (es?.readyState === 2) refresh() }
  es.addEventListener('item.updated', (ev: any) => {
    const d = JSON.parse(ev.data)
    if (d.deleted) items.value = items.value.filter(i => i.id !== d.item_id)
    else items.value = items.value.map(i => i.id === d.item_id ? { ...i, ...d, id: i.id, progress_percent: Math.min(100, Math.round(d.qty_claimed / d.qty_needed * 100)) } : i)
  })
  es.addEventListener('wishlist.updated', () => refresh())
})
onBeforeUnmount(() => es?.close())

// --- 認領 sheet / 成功 overlay ---
const sheetItem = ref<any>(null), done = ref<any>(null), reporting = ref(false), toast = ref('')
const sheetMine = computed(() => sheetItem.value ? myClaims.value[sheetItem.value.id] ?? null : null)
function onDone(r: any) {
  myClaims.value = { ...myClaims.value, [r.claim.item_id]: r.claim }
  items.value = items.value.map(i => i.id === r.item.id ? { ...i, ...r.item, is_fully_claimed: r.item.qty_remaining <= 0, progress_percent: Math.min(100, Math.round(r.item.qty_claimed / r.item.qty_needed * 100)) } : i)
  done.value = { ...r, title: sheetItem.value.title }
  sheetItem.value = null; refresh()
}
async function onSwitchEdit() { await loadMine(); const it = sheetItem.value; sheetItem.value = null; await nextTick(); sheetItem.value = it }
function onStale(rem: number) { items.value = items.value.map(i => i.id === sheetItem.value?.id ? { ...i, qty_remaining: rem, is_fully_claimed: rem <= 0 } : i) }
const maskedEmail = (e: string) => e.replace(/^(.).*(@.*)$/, '$1***$2')
function dismissHint() { setPref('ws_hint_dismissed', '1'); lineHint.value = false }
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <div v-if="gone" class="g-center"><h1>此清單已被下架</h1><p class="g-mute">這份清單因違反服務條款已被移除，目前無法瀏覽或認領。</p><NuxtLink to="/">回首頁</NuxtLink></div>
    <div v-else-if="!data" class="g-center"><h1>找不到這份清單</h1><NuxtLink to="/">回首頁</NuxtLink></div>
    <template v-else>
      <div v-if="!online" class="g-banner">目前離線，顯示的是上次載入的內容，認領功能暫停。</div>
      <div v-if="lineHint" class="g-banner">想把認領保存起來？點右上角 ⋯ 選「用預設瀏覽器開啟」，之後比較不會找不到。 <button class="g-link" @click="dismissHint">知道了</button></div>
      <div v-if="closed" class="g-banner">這份清單已結束</div>
      <img v-if="data.cover_image_url" :src="data.cover_image_url" alt="" class="g-cover">
      <h1>{{ data.title }}</h1>
      <p class="g-mute">
        <template v-if="data.event_date">日期 {{ data.event_date }}・</template>由 {{ data.owner.display_name }} 建立
      </p>
      <p v-if="data.description">{{ data.description }}</p>
      <div v-if="masked" class="g-banner">你正以建立者身分瀏覽，解鎖前不顯示各品項進度。</div>
      <template v-else>
        <div class="g-mute">完成度 {{ pct }}%・{{ data.completion.fulfilled_count }} / {{ data.completion.item_count }} 項</div>
        <div class="g-bar" role="progressbar" :aria-valuenow="pct" aria-valuemin="0" aria-valuemax="100"><i :style="{ width: pct + '%' }" /></div>
      </template>
      <p v-if="!items.length" class="g-center g-mute">建立者還在準備清單，晚點再回來看看</p>
      <ul class="c-list">
        <GuestItemCard v-for="it in sorted" :key="it.id" :item="it" :mine="myClaims[it.id] ?? null" :masked="masked" :closed="closed" :online="online" @claim="sheetItem = it" />
      </ul>
      <div class="g-foot">
        <div><span class="g-dot" :class="{ on: live }" />{{ live ? '即時更新中' : '連線中…' }}</div>
        <button class="g-link" @click="reporting = true">檢舉此清單</button>
        ・<NuxtLink to="/terms">服務條款</NuxtLink>・<NuxtLink to="/privacy">隱私權政策</NuxtLink>
      </div>
    </template>

    <GuestClaimSheet v-if="sheetItem" :key="sheetItem.id + (sheetMine?.id ?? '')" :item="sheetItem" :mine="sheetMine" :online="online" :logged-in-name="me?.display_name"
      @close="sheetItem = null" @done="onDone" @stale="onStale" @switch-edit="onSwitchEdit" />
    <GuestReportSheet v-if="reporting" :slug="slug" @close="reporting = false" @sent="reporting = false; toast = '已收到檢舉，我們會盡快處理'" />

    <div v-if="done" class="g-mask"><div class="g-sheet center" role="dialog" aria-modal="true">
      <h1 class="ok">✓ 認領成功！</h1>
      <p>你認領了 {{ done.title }} × {{ done.claim.qty }}<template v-if="done.name || done.claim.claimer_name">，謝謝你，{{ done.claim.claimer_name }}</template></p>
      <div v-if="!done.persisted" class="g-banner err">無法儲存於此裝置，請截圖保存。換瀏覽器後可能找不到這份認領。</div>
      <div v-else class="g-banner ok">這份認領已存在此裝置，可在「我的認領」查看、標記已購買或取消。</div>
      <p v-if="done.email" class="g-mute">已寄出管理連結到 {{ maskedEmail(done.email) }}</p>
      <div v-if="isLineBrowser()" class="g-banner">LINE 內建瀏覽器關閉後，認領紀錄可能找不到。點右上角 ⋯ →「用預設瀏覽器開啟」。</div>
      <NuxtLink class="g-btn block" to="/me/claims">查看我的認領</NuxtLink>
      <button class="g-link" @click="done = null">回到清單繼續看</button>
    </div></div>
    <div v-if="toast" class="g-banner g-toast" role="status" @click="toast = ''">{{ toast }}</div>
  </main>
</template>
