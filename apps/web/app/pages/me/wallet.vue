<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
useHead({ title: '我的點數' })
const { api, base } = useApi()
const wallet = ref<any>(null), ledger = ref<any[]>([]), lCursor = ref<string | null>(null)
const contribs = ref<any[]>([]), cCursor = ref<string | null>(null)
const loading = ref(true), err = ref(''), msg = ref('')

async function loadWallet(more = false) {
  const r = await api('/wallet', { query: { limit: 20, cursor: more ? lCursor.value : undefined } })
  wallet.value = r.wallet
  ledger.value = more ? [...ledger.value, ...r.data] : r.data; lCursor.value = r.next_cursor
}
async function loadContribs(more = false) {
  const r = await api('/wallet/contributions', { query: { limit: 20, cursor: more ? cCursor.value : undefined } })
  contribs.value = more ? [...contribs.value, ...r.data] : r.data; cCursor.value = r.next_cursor
}
async function load() {
  err.value = ''
  try { await Promise.all([loadWallet(), loadContribs()]) } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
onMounted(load)
const more = (f: () => Promise<void>) => f().catch((e) => { err.value = errMsg(e) })

// 撤回
const wd = ref<any>(null), wdBusy = ref(false), wdErr = ref('')
async function withdraw() {
  wdBusy.value = true; wdErr.value = ''
  try {
    await api(`/contributions/${wd.value.contribution.id}`, { method: 'DELETE' })
    msg.value = `已撤回 ${fmtPts(wd.value.contribution.points)} 點`; wd.value = null; await load()
  } catch (e: any) {
    wdErr.value = e.code === 'CONTRIBUTION_LOCKED' ? '這筆認捐已無法撤回（品項已達標或超過撤回期限）' : errMsg(e)
    if (e.code === 'CONTRIBUTION_LOCKED') await load()
  } finally { wdBusy.value = false }
}

// 轉投：目標限同一份清單、仍在募集中的其他眾籌品項（從公開頁資料取得）
const re = ref<any>(null), reTargets = ref<any[]>([]), reTarget = ref(''), reBusy = ref(false), reErr = ref(''), reKey = ref('')
async function openRe(row: any) {
  re.value = row; reTargets.value = []; reTarget.value = ''; reErr.value = ''; reKey.value = crypto.randomUUID()
  try {
    const w = await $fetch<any>(`${base}/public/wishlists/${row.wishlist.slug}`, { credentials: 'omit' })
    reTargets.value = (w.items || []).filter((i: any) => i.funding_mode === 'crowdfund' && i.id !== row.item.id && deriveDisplayStatus(i) === 'open')
    reTarget.value = reTargets.value[0]?.id ?? ''
  } catch { reErr.value = '讀取同清單的其他品項失敗，請稍後再試' }
}
async function reallocate() {
  reBusy.value = true; reErr.value = ''
  try {
    await api(`/contributions/${re.value.contribution.id}/reallocate`, { method: 'POST', headers: { 'Idempotency-Key': reKey.value }, body: { target_item_id: reTarget.value } })
    msg.value = '已轉投'; re.value = null; await load()
  } catch (e: any) {
    reErr.value = e.code === 'REALLOCATION_NOT_ALLOWED' ? '無法轉投到這個品項（可能已達標、已截止或超過選擇期限），請換一個再試' : errMsg(e)
    if (e.code !== 'NETWORK') reKey.value = crypto.randomUUID()
  } finally { reBusy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="我的點數" back="/dashboard" />
    <p v-if="loading" class="c-mute" role="status">載入中…</p>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load">重試</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <template v-if="wallet">
      <section class="c-card c-center">
        <div class="c-big">{{ fmtPts(wallet.balance) }}</div>
        <p class="c-mute c-m0">可用點數（1 點 = NT$1）<span v-if="wallet.status === 'frozen'" class="c-badge frozen">已凍結</span></p>
        <p class="c-mute">認捐中 {{ fmtPts(wallet.held_points) }} 點（已認捐，達標前可撤回）</p>
        <p v-if="wallet.status === 'frozen'" class="c-err" role="alert">錢包已被凍結，暫時無法贊助，請聯絡客服。</p>
        <p class="c-mute c-small">目前點數由平台發放。</p>
        <button class="c-btn" disabled>儲值（即將推出）</button>
      </section>

      <h2 class="c-h2">我的認捐</h2>
      <p v-if="!contribs.length" class="c-center c-mute">還沒有認捐紀錄。打開朋友分享的清單，就能用點數贊助。</p>
      <article v-for="r in contribs" :key="r.contribution.id" class="c-card">
        <div class="c-row">
          <img v-if="r.item.image_url" class="c-thumb" :src="r.item.image_url" alt="">
          <div v-else class="c-thumb">無圖</div>
          <div class="c-grow">
            <strong>{{ r.item.title }}</strong>
            <div class="c-mute"><NuxtLink :to="`/s/${r.wishlist.slug}`">{{ r.wishlist.title }}</NuxtLink></div>
            <div>{{ fmtPts(r.contribution.points) }} 點　<span class="c-badge" :class="r.contribution.status">{{ label(CONTRIBUTION_STATUS, r.contribution.status) }}</span>
              <span v-if="r.item.display_status" class="c-badge" :class="r.item.display_status">{{ displayStatusLabel(r.item.display_status) }}</span></div>
          </div>
        </div>
        <div v-if="r.contribution.message" class="c-mute">「{{ r.contribution.message }}」{{ r.contribution.is_anonymous ? '（匿名）' : '' }}</div>
        <div class="c-mute c-small">認捐 {{ fmtTime(r.contribution.created_at) }}<template v-if="r.contribution.captured_at">・達標 {{ fmtTime(r.contribution.captured_at) }}</template></div>
        <div v-if="r.contribution.refunded_points" class="c-mute c-small">已退回差額 {{ fmtPts(r.contribution.refunded_points) }} 點</div>
        <div v-if="r.item.display_status === 'expired' && r.item.reallocation_deadline && (r.can_withdraw || r.can_reallocate)" class="c-err c-small">募集已截止，請在 {{ fmtTime(r.item.reallocation_deadline) }} 前撤回或轉投，逾期會自動退回。</div>
        <div v-if="r.can_withdraw || r.can_reallocate" class="c-row wrap c-mt">
          <button v-if="r.can_reallocate" class="c-btn c-grow" @click="openRe(r)">轉投其他品項</button>
          <button v-if="r.can_withdraw" class="c-btn danger c-grow" @click="wd = r; wdErr = ''">撤回</button>
        </div>
      </article>
      <button v-if="cCursor" class="c-btn block c-mb16" @click="more(() => loadContribs(true))">載入更多</button>

      <h2 class="c-h2">點數帳本</h2>
      <p v-if="!ledger.length" class="c-center c-mute">還沒有異動紀錄</p>
      <section v-else class="c-card">
        <ul class="c-list">
          <li v-for="e in ledger" :key="e.id" class="c-row wrap">
            <span class="c-grow"><strong>{{ label(LEDGER_TYPE, e.entry_type) }}</strong><span v-if="e.note" class="c-mute">　{{ e.note }}</span>
              <span class="c-mute c-small" style="display:block">{{ fmtTime(e.created_at) }}・餘額 {{ fmtPts(e.balance_after) }}</span></span>
            <b :class="e.delta >= 0 ? 'c-pos' : 'c-neg'">{{ e.delta > 0 ? '+' : '' }}{{ fmtPts(e.delta) }}</b>
          </li>
        </ul>
        <button v-if="lCursor" class="c-btn block c-mt" @click="more(() => loadWallet(true))">載入更多</button>
      </section>
    </template>

    <CreatorConfirm :open="!!wd" title="撤回認捐" :text="`撤回「${wd?.item.title}」的 ${fmtPts(wd?.contribution.points)} 點？點數會立即退回你的錢包。`" ok="確認撤回" danger :busy="wdBusy" :error="wdErr" @close="wd = null" @ok="withdraw" />
    <CreatorConfirm :open="!!re" title="轉投其他品項" :text="`把 ${fmtPts(re?.contribution.points)} 點轉投到同一份清單的其他眾籌品項（不會異動錢包）。`" ok="確認轉投" :busy="reBusy" :disabled="!reTarget" :error="reErr" @close="re = null" @ok="reallocate">
      <p v-if="re && !reTargets.length && !reErr" class="c-mute">這份清單目前沒有其他募集中的眾籌品項。</p>
      <label v-if="reTargets.length" class="c-field"><span>轉投到</span>
        <select v-model="reTarget"><option v-for="t in reTargets" :key="t.id" :value="t.id">{{ t.title }}（還差 {{ fmtPts(t.remaining_points) }} 點）</option></select></label>
    </CreatorConfirm>
  </main>
</template>
