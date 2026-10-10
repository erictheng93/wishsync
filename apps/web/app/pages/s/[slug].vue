<script setup lang="ts">
import '~/assets/guest.css'
// 公開分享頁：必須 SSR，LINE / FB 爬蟲才讀得到 OG（docs 03 §5.6）
const route = useRoute()
const slug = route.params.slug as string
const { public: { apiBase } } = useRuntimeConfig()
const { api } = useGuest()

// 不用 useFetch 的 error 拋出：404 / 410 在頁內顯示，並設正確 HTTP 狀態碼（410 需 noindex、不洩漏標題）
// cache: 'no-cache'：API 回的 Cache-Control 帶 stale-while-revalidate（給 CDN 用），瀏覽器會先吐舊資料；認領後的 refresh() 必須是新的（仍會用 ETag 回 304）
const { data: fetched, error, refresh } = await useFetch<any>(`${apiBase}/api/v1/public/wishlists/${slug}`, { credentials: 'omit', cache: 'no-cache' })
// 404 找不到 / 410 下架 / 其他（5xx、網路、逾時）= 暫時無法載入，回 503，避免爬蟲與快取把暫時性錯誤記成「不存在」
// 暫時性錯誤（5xx / 網路）時保留上次成功的內容，不要讓 15 秒輪詢的一次失敗把整頁換成錯誤畫面；404 / 410 才清空
const data = ref<any>(fetched.value)
watch([fetched, error], ([v, e]) => { if (v) data.value = v; else if (e && [404, 410].includes(e.statusCode as number)) data.value = null })
const gone = computed(() => error.value?.statusCode === 410)
const notFound = computed(() => error.value?.statusCode === 404)
const failed = computed(() => !!error.value && !gone.value && !notFound.value)
if (error.value && import.meta.server) setResponseStatus(useRequestEvent()!, gone.value ? 410 : notFound.value ? 404 : 503)

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
} else useSeoMeta({ title: gone.value ? '此清單已被下架' : failed.value ? '暫時無法載入' : '找不到這份清單', robots: 'noindex' })

useHead({ noscript: [{ innerHTML: '<style>.js-claim{display:none!important}.g-nojs{display:block!important}</style>' }] })
const items = ref<any[]>(data.value?.items ?? [])
watch(data, v => { if (v) items.value = v.items })
// 剩餘優先 → 已滿排最後（穩定排序）
const isDone = (i: any) => i.funding_mode === 'crowdfund' ? deriveDisplayStatus(i) !== 'open' : i.is_fully_claimed
const sorted = computed(() => [...items.value].sort((a, b) => Number(isDone(a)) - Number(isDone(b))))
const pct = computed(() => data.value?.completion?.completion_pct ?? 0)
const closed = computed(() => data.value?.status === 'closed')

// --- 線上狀態 / LINE 引導 ---
const online = ref(true), mode = ref<LiveMode>('connecting'), lineHint = ref(false)
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

let live: ReturnType<typeof createLiveRefresh> | undefined
onMounted(async () => {
  online.value = navigator.onLine
  addEventListener('online', () => { online.value = true; refresh() }); addEventListener('offline', () => (online.value = false))
  lineHint.value = isLineBrowser() && !getPref('ws_hint_dismissed')
  loadMine()
  $fetch<any>(`${apiBase}/api/v1/me`, { credentials: 'include' }).then(m => { me.value = m; isOwner.value = m?.id === data.value?.owner?.id }).catch(() => {})
  if (!data.value) return
  live = createLiveRefresh({
    url: `${apiBase}/api/v1/public/wishlists/${slug}/events`,
    refresh, onMode: m => (mode.value = m),
    onEvent: (type, d) => {
      if (type === 'wishlist.updated' || !d) return refresh()
      if (d.deleted) items.value = items.value.filter(i => i.id !== d.item_id)
      else {
        items.value = items.value.map(i => i.id === d.item_id ? mergeItemEvent(i, d) : i)
        if (items.value.some(i => i.id === d.item_id && i.funding_mode === 'crowdfund')) refresh() // contributors 不在事件裡，補抓一次
      }
    },
  })
  live.start()
})
onBeforeUnmount(() => live?.stop())


// --- 認領 sheet / 成功 overlay ---
const sheetItem = ref<any>(null), done = ref<any>(null), reporting = ref(false), toast = ref('')
const sheetMine = computed(() => sheetItem.value ? myClaims.value[sheetItem.value.id] ?? null : null)
function onDone(r: any) {
  myClaims.value = { ...myClaims.value, [r.claim.item_id]: r.claim }
  items.value = items.value.map(i => i.id === r.item.id ? { ...i, ...r.item, is_fully_claimed: r.item.qty_remaining <= 0, progress_percent: Math.min(100, Math.round(r.item.qty_claimed / r.item.qty_needed * 100)) } : i)
  done.value = { ...r, title: sheetItem.value.title }
  sheetItem.value = null; refresh()
}
// --- 點數贊助（需登入：未登入導到登入頁，登入後回到本頁）---
const fundId = ref<string | null>(null), funded = ref<any>(null)
const fundItem = computed(() => items.value.find(i => i.id === fundId.value) ?? null) // 取即時資料（SSE / refresh 會換掉物件）
function onFund(it: any) {
  if (!me.value) return navigateTo({ path: '/login', query: { redirect: `/s/${slug}` } })
  fundId.value = it.id
}
function onFunded(r: any) {
  items.value = items.value.map(i => i.id === r.item.id ? { ...i, ...r.item } : i)
  funded.value = { ...r, title: fundItem.value?.title }
  fundId.value = null; refresh()
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
    <div v-else-if="failed && !data" class="g-center"><h1>系統暫時無法載入，請稍後再試</h1><button class="g-btn" @click="refresh()">重新載入</button></div>
    <div v-else-if="!data" class="g-center"><h1>找不到這份清單</h1><NuxtLink to="/">回首頁</NuxtLink></div>
    <template v-else>
      <div class="g-banner g-nojs">需要 JavaScript 才能認領；你仍可查看清單內容</div>
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
        <GuestItemCard v-for="it in sorted" :key="it.id" :item="it" :mine="myClaims[it.id] ?? null" :masked="masked" :closed="closed" :online="online" @claim="sheetItem = it" @fund="onFund(it)" />
      </ul>
      <div class="g-foot">
        <div><span class="g-dot" :class="{ on: mode !== 'connecting' }" />{{ liveLabel(mode) }}</div>
        <button class="g-link" @click="reporting = true">檢舉此清單</button>
        ・<NuxtLink to="/terms">服務條款</NuxtLink>・<NuxtLink to="/privacy">隱私權政策</NuxtLink>
      </div>
    </template>

    <GuestClaimSheet v-if="sheetItem" :key="sheetItem.id + (sheetMine?.id ?? '')" :item="sheetItem" :mine="sheetMine" :online="online" :logged-in-name="me?.display_name"
      @close="sheetItem = null" @done="onDone" @stale="onStale" @switch-edit="onSwitchEdit" />
    <GuestFundSheet v-if="fundItem" :key="fundItem.id" :item="fundItem" :slug="slug" :online="online" :logged-in-name="me?.display_name"
      @close="fundId = null" @done="onFunded" @stale="refresh()" />
    <GuestDialog v-if="funded" labelledby="funded-title" @close="funded = null"><div class="g-sheet center">
      <h1 id="funded-title" class="ok">✓ 感謝你的贊助！</h1>
      <p>你為「{{ funded.title }}」贊助了 {{ fmtPts(funded.contribution.points) }} 點</p>
      <div v-if="funded.funded" class="g-banner ok">剛好達標了！平台會代購並寄出，進度會用 Email 通知你。</div>
      <p v-if="funded.wallet" class="g-mute">錢包餘額剩 {{ fmtPts(funded.wallet.balance) }} 點</p>
      <NuxtLink class="g-btn block" to="/me/wallet">查看我的點數與認捐</NuxtLink>
      <button class="g-link" @click="funded = null">回到清單繼續看</button>
    </div></GuestDialog>
    <GuestReportSheet v-if="reporting" :slug="slug" @close="reporting = false" @sent="reporting = false; toast = '已收到檢舉，我們會盡快處理'" />

    <GuestDialog v-if="done" labelledby="done-title" @close="done = null"><div class="g-sheet center">
      <h1 id="done-title" class="ok">✓ 認領成功！</h1>
      <p>你認領了 {{ done.title }} × {{ done.claim.qty }}<template v-if="done.name || done.claim.claimer_name">，謝謝你，{{ done.claim.claimer_name }}</template></p>
      <div v-if="done.lost" class="g-banner err" role="alert">認領已送出，但這個瀏覽器沒有保存到身分，之後可能無法在這裡修改或取消。請先截圖保存；若有填 Email，可用確認信中的「管理我的認領」連結找回，也可以重新整理頁面確認。</div>
      <div v-else-if="!done.persisted" class="g-banner err">無法儲存於此裝置，請截圖保存。換瀏覽器後可能找不到這份認領。</div>
      <div v-else class="g-banner ok">這份認領已存在此裝置，可在「我的認領」查看、標記已購買或取消。</div>
      <p v-if="done.email" class="g-mute">已寄出管理連結到 {{ maskedEmail(done.email) }}</p>
      <div v-if="isLineBrowser()" class="g-banner">LINE 內建瀏覽器關閉後，認領紀錄可能找不到。點右上角 ⋯ →「用預設瀏覽器開啟」。</div>
      <NuxtLink class="g-btn block" to="/me/claims">查看我的認領</NuxtLink>
      <button class="g-link" @click="done = null">回到清單繼續看</button>
    </div></GuestDialog>
    <div v-if="toast" class="g-banner g-toast" role="status" @click="toast = ''">{{ toast }}</div>
  </main>
</template>
