<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
useHead({ title: '完成度' })
const id = useRoute().params.id as string
const { api } = useApi()
const d = ref<any>(null)
const w = ref<any>(null)
const orders = ref<any>(null)
const err = ref(''), loading = ref(true), updatedAt = ref('')
const slug = computed(() => w.value?.slug)
let tm: any, countdown: any
const left = ref(0)

async function load() {
  try {
    if (!w.value) w.value = (await api(`/wishlists/${id}`)).wishlist
    d.value = await api(`/wishlists/${id}/dashboard`)
    // 採購單進度：只有含眾籌品項才需要；失敗不影響其他區塊（鎖定時 data 為 null，只有彙總）
    orders.value = (d.value.items || []).some((i: any) => i.funding_mode === 'crowdfund') ? await api(`/wishlists/${id}/orders`).catch(() => null) : null
    updatedAt.value = new Date().toLocaleTimeString('zh-TW')
    err.value = ''
    startCountdown()
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
function startCountdown() {
  clearInterval(countdown)
  if (!d.value?.surprise_locked || !d.value.unlock_at) return
  const end = Date.parse(d.value.unlock_at)
  const f = () => { left.value = end - Date.now(); if (left.value <= 0) { clearInterval(countdown); load() } } // 解鎖後自動重載
  f(); countdown = setInterval(f, 1000)
}
const days = computed(() => Math.floor(left.value / 864e5))
const hms = computed(() => { const s = Math.max(0, Math.floor(left.value / 1000)) % 86400; return `${Math.floor(s / 3600)} 小時 ${Math.floor(s % 3600 / 60)} 分 ${s % 60} 秒` })

// SSE 事件合併重取（debounce 500ms）
const { status } = useWishlistEvents(slug, () => { clearTimeout(tm); tm = setTimeout(load, 500) })
onMounted(load)
onBeforeUnmount(() => { clearTimeout(tm); clearInterval(countdown) })

const locked = computed(() => !!d.value?.surprise_locked)
const claimsByItem = computed(() => {
  const m: Record<string, any[]> = {}
  for (const c of d.value?.claims || []) (m[c.item_id] ||= []).push(c)
  return m
})
const contribsByItem = computed(() => {
  const m: Record<string, any[]> = {}
  for (const c of d.value?.contributions || []) (m[c.item_id] ||= []).push(c)
  return m
})
const orderByItem = computed(() => Object.fromEntries((orders.value?.data || []).map((o: any) => [o.item_id, o])))
const fundPct = (i: any) => i.target_points > 0 ? Math.min(100, Math.floor(i.pledged_points / i.target_points * 100)) : 0
const fundStatus = (i: any) => deriveDisplayStatus(i, orderByItem.value[i.item_id])
const pct = computed(() => d.value?.totals?.completion_pct ?? 0)
const busyClaim = ref('')
const claimErr = ref('')
async function setStatus(c: any, status: 'delivered' | 'cancelled') {
  if (status === 'cancelled' && !confirm(`確定要取消「${c.claimer_name || '訪客'}」的認領嗎？數量會釋出給其他人。`)) return
  busyClaim.value = c.id; claimErr.value = ''
  try { await api(`/claims/${c.id}`, { method: 'PATCH', body: { status } }); await load() }
  catch (e) { claimErr.value = errMsg(e) } finally { busyClaim.value = '' }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="w?.title ? `完成度・${w.title}` : '完成度'" :back="`/lists/${id}/edit`">
      <button class="c-btn" aria-label="重新整理" @click="load">↻</button>
    </CreatorHeader>
    <p v-if="loading" class="c-mute" role="status">載入中…</p>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load">重試</button></p>
    <template v-if="d">
      <section class="c-card c-center">
        <div class="c-big">{{ pct }}%</div>
        <div class="c-bar" role="progressbar" :aria-valuenow="pct" aria-valuemin="0" aria-valuemax="100" aria-label="整體完成度"><i :style="{ width: pct + '%' }" /></div>
        <p class="c-mute">完成 {{ d.totals.fulfilled_count }} / {{ d.totals.item_count }} 項<template v-if="!locked">・數量 {{ d.totals.qty_claimed }} / {{ d.totals.qty_needed }}</template></p>
        <p v-if="!locked && d.totals.target_points" class="c-mute">點數 {{ fmtPts(d.totals.pledged_points) }} / {{ fmtPts(d.totals.target_points) }}・已達標 {{ d.totals.funded_item_count ?? 0 }} 項</p>
        <p class="c-mute c-small">{{ status === 'live' ? '即時更新中' : status === 'polling' ? '定時更新中' : '' }}　更新於 {{ updatedAt }}</p>
      </section>

      <section v-if="locked" class="c-mask" role="status">
        <strong>驚喜模式</strong>
        <p>解鎖前只顯示整體完成度，不顯示各品項進度與誰送了什麼。</p>
        <p>還有 {{ days }} 天 {{ hms }} 解鎖（{{ new Date(d.unlock_at).toLocaleString('zh-TW', { timeZone: 'Asia/Taipei' }) }} 台北時間）</p>
      </section>

      <p v-if="!d.totals.item_count" class="c-center c-mute">尚無品項</p>
      <p v-else-if="!locked && !d.totals.qty_claimed && !d.totals.pledged_points" class="c-center c-mute">還沒有人認領或贊助，去分享連結吧</p>
      <p v-if="claimErr" class="c-err" role="alert">{{ claimErr }}</p>

      <h2 class="c-h2">品項明細</h2>
      <article v-for="i in d.items" :key="i.item_id" class="c-card">
        <template v-if="i.funding_mode === 'crowdfund'">
          <div class="c-row"><strong class="c-grow">{{ i.title }}</strong>
            <span v-if="i.pledged_points === null" class="c-mute">進度已隱藏</span>
            <span v-else-if="fundStatus(i)" class="c-badge" :class="fundStatus(i)">{{ displayStatusLabel(fundStatus(i)) }}</span>
          </div>
          <template v-if="i.pledged_points !== null">
            <div class="c-bar" role="progressbar" :aria-valuenow="fundPct(i)" aria-valuemin="0" aria-valuemax="100" :aria-label="i.title"><i :style="{ width: fundPct(i) + '%' }" /></div>
            <p class="c-mute c-m0">已募 {{ fmtPts(i.pledged_points) }} / {{ fmtPts(i.target_points) }} 點<template v-if="i.funding_deadline && fundStatus(i) === 'open'">・截止 {{ fmtTime(i.funding_deadline) }}</template></p>
          </template>
          <div v-if="!locked" class="c-sub">
            <strong class="c-small">捐款者</strong>
            <p v-if="!(contribsByItem[i.item_id] || []).length" class="c-mute c-m0">還沒有人贊助</p>
            <ul v-else class="c-list">
              <li v-for="c in contribsByItem[i.item_id]" :key="c.id">
                <div class="c-row wrap">
                  <span class="c-grow"><strong>{{ c.display_name }}</strong>　{{ fmtPts(c.points) }} 點</span>
                  <span class="c-badge" :class="c.status">{{ label(CONTRIBUTION_STATUS, c.status) }}</span>
                </div>
                <div v-if="c.message" class="c-mute">「{{ c.message }}」</div>
                <div class="c-mute c-small">認捐時間 {{ fmtTime(c.created_at) }}<template v-if="c.captured_at">・達標時間 {{ fmtTime(c.captured_at) }}</template></div>
                <div v-if="c.refunded_points" class="c-mute c-small">已退回 {{ fmtPts(c.refunded_points) }} 點（實際扣用 {{ fmtPts(c.spent_points ?? c.points - c.refunded_points) }} 點）</div>
              </li>
            </ul>
            <template v-if="orderByItem[i.item_id]">
              <strong class="c-small">採購進度</strong>
              <dl class="c-dl">
                <dt>狀態</dt><dd>{{ label(ORDER_STATUS, orderByItem[i.item_id].status) }}</dd>
                <template v-if="orderByItem[i.item_id].tracking_no"><dt>物流單號</dt><dd>{{ orderByItem[i.item_id].tracking_no }}</dd></template>
                <template v-if="orderByItem[i.item_id].placed_at"><dt>下單時間</dt><dd>{{ fmtTime(orderByItem[i.item_id].placed_at) }}</dd></template>
                <template v-if="orderByItem[i.item_id].shipped_at"><dt>出貨時間</dt><dd>{{ fmtTime(orderByItem[i.item_id].shipped_at) }}</dd></template>
                <template v-if="orderByItem[i.item_id].delivered_at"><dt>送達時間</dt><dd>{{ fmtTime(orderByItem[i.item_id].delivered_at) }}</dd></template>
              </dl>
            </template>
            <p v-else-if="fundStatus(i) === 'funded'" class="c-mute c-small">已達標，等待平台下單。</p>
          </div>
        </template>
        <template v-else>
        <div class="c-row"><strong class="c-grow">{{ i.title }}</strong>
          <span v-if="i.qty_claimed === null" class="c-mute">進度已隱藏</span>
          <span v-else-if="i.qty_claimed >= i.qty_needed" class="c-badge active">已完成</span>
          <span v-else>{{ i.qty_claimed }} / {{ i.qty_needed }}</span>
        </div>
        <div v-if="i.qty_claimed !== null" class="c-bar" role="progressbar" :aria-valuenow="Math.min(100, Math.round(i.qty_claimed / i.qty_needed * 100))" aria-valuemin="0" aria-valuemax="100" :aria-label="i.title"><i :style="{ width: Math.min(100, i.qty_claimed / i.qty_needed * 100) + '%' }" /></div>
        <ul v-for="c in claimsByItem[i.item_id] || []" :key="c.id" class="c-list">
          <li class="c-row wrap">
            <span class="c-grow" style="flex-basis:100%">{{ c.claimer_name || '訪客' }} ×{{ c.qty }}<template v-if="c.note"> ・{{ c.note }}</template></span>
            <CreatorStatusBadge :v="c.status" />
            <template v-if="c.status === 'reserved' || c.status === 'purchased'">
              <button class="c-btn" :disabled="busyClaim === c.id" @click="setStatus(c, 'delivered')">標為已送達</button>
              <button class="c-btn danger" :disabled="busyClaim === c.id" @click="setStatus(c, 'cancelled')">取消認領</button>
            </template>
          </li>
        </ul>
        </template>
      </article>
      <section v-if="d.orders_summary && (d.orders_summary.funded_count || d.orders_summary.ordered_count || d.orders_summary.shipped_count || d.orders_summary.delivered_count)" class="c-card">
        <strong>眾籌採購彙總</strong>
        <p class="c-mute">已達標 {{ d.orders_summary.funded_count }}・已下單 {{ d.orders_summary.ordered_count }}・運送中 {{ d.orders_summary.shipped_count }}・已送達 {{ d.orders_summary.delivered_count }}</p>
      </section>
      <NuxtLink v-if="!d.totals.qty_claimed && !d.totals.pledged_points && !locked" :to="`/lists/${id}/edit`" class="c-btn primary block">分享清單</NuxtLink>
    </template>
  </main>
</template>
