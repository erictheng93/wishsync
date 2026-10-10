<script setup lang="ts">
definePageMeta({ middleware: 'staff' })
useHead({ title: '點數錢包' })
const { api } = useApi()
const q = ref(''), rows = ref<ReturnType<typeof normWalletRow>[]>([]), done = ref(false)
const err = ref(''), msg = ref('')
async function search() {
  err.value = msg.value = ''
  try { rows.value = (await api('/admin/wallets', { query: { q: q.value.trim() || undefined } })).data.map(normWalletRow); done.value = true }
  catch (e) { err.value = errMsg(e) }
}

// 發點 / 扣點（二次確認；Idempotency-Key 於開啟確認時產生，失敗重試沿用）
const g = reactive({ points: '' as string | number, reason: '' })
const target = ref<any>(null), gAsk = ref(false), gBusy = ref(false), gErr = ref(''), gKey = ref('')
function ask(r: any) { target.value = r; g.points = ''; g.reason = ''; gErr.value = '' }
function confirmGrant() {
  gErr.value = grantError(g.points, g.reason)
  if (gErr.value) return
  gKey.value = crypto.randomUUID(); gAsk.value = true
}
async function grant() {
  gBusy.value = true; gErr.value = ''
  try {
    const r = await api('/admin/wallets/grants', { method: 'POST', headers: { 'Idempotency-Key': gKey.value }, body: { user_id: target.value.user_id, points: Number(g.points), reason: g.reason.trim() } })
    Object.assign(target.value, { balance: r.wallet.balance, status: r.wallet.status ?? target.value.status, wallet_id: r.wallet.id ?? target.value.wallet_id })
    msg.value = `已${Number(g.points) > 0 ? '發放' : '扣除'} ${fmtPts(Math.abs(Number(g.points)))} 點，${target.value.display_name} 目前餘額 ${fmtPts(r.wallet.balance)} 點`
    gAsk.value = false; target.value = null
  } catch (e: any) {
    gErr.value = e.code === 'INSUFFICIENT_POINTS' ? `扣點後會變成負數（目前餘額 ${fmtPts(e.body?.balance)} 點）` : errMsg(e)
    gAsk.value = false
    if (e.code !== 'NETWORK') gKey.value = crypto.randomUUID()
  } finally { gBusy.value = false }
}

// 凍結 / 解凍
const fz = ref<any>(null), fzReason = ref(''), fzBusy = ref(false), fzErr = ref('')
async function setStatus() {
  fzBusy.value = true; fzErr.value = ''
  const to = fz.value.status === 'frozen' ? 'active' : 'frozen'
  try {
    const r = await api(`/admin/wallets/${fz.value.wallet_id}`, { method: 'PATCH', body: { status: to, reason: fzReason.value.trim() } })
    fz.value.status = r.status ?? r.wallet?.status ?? to
    msg.value = `${fz.value.display_name} 的錢包已${to === 'frozen' ? '凍結' : '解凍'}`; fz.value = null
  } catch (e) { fzErr.value = errMsg(e) } finally { fzBusy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader title="點數錢包" back="/admin" />
    <form class="c-row c-mb" @submit.prevent="search"><input v-model="q" class="c-grow c-input" placeholder="Email 或顯示名稱" aria-label="搜尋使用者"><button class="c-btn primary">搜尋</button></form>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="done && !rows.length" class="c-center c-mute">沒有符合的使用者</p>
    <article v-for="r in rows" :key="r.user_id" class="c-card" :class="{ bad: r.status === 'frozen' }">
      <div class="c-row"><strong class="c-grow">{{ r.display_name }}</strong><span v-if="r.status === 'frozen'" class="c-badge frozen">已凍結</span></div>
      <div class="c-mute">{{ r.email }}</div>
      <div>餘額 <b>{{ fmtPts(r.balance) }}</b> 點</div>
      <div class="c-row wrap c-mt">
        <button class="c-btn primary c-grow" @click="ask(r)">發點 / 扣點</button>
        <button v-if="r.wallet_id" class="c-btn c-grow" :class="{ danger: r.status !== 'frozen' }" @click="fz = r; fzReason = ''; fzErr = ''">{{ r.status === 'frozen' ? '解凍' : '凍結' }}</button>
      </div>
    </article>

    <CreatorSheet :open="!!target" title="發點 / 扣點" @close="target = null">
      <p>對象：<strong>{{ target?.display_name }}</strong>（{{ target?.email }}）　目前餘額 {{ fmtPts(target?.balance) }} 點</p>
      <form @submit.prevent="confirmGrant">
        <label class="c-field"><span>點數（正數發放、負數扣除，單次 ±1,000,000 以內）</span><input v-model="g.points" type="number" inputmode="numeric" step="1" required></label>
        <label class="c-field"><span>原因（必填，會寫入帳本與稽核紀錄）</span><input v-model="g.reason" maxlength="200" required></label>
        <p v-if="gErr" class="c-err" role="alert">{{ gErr }}</p>
        <div class="c-row">
          <button type="button" class="c-btn c-grow" @click="target = null">取消</button>
          <button class="c-btn primary c-grow">下一步</button>
        </div>
      </form>
    </CreatorSheet>
    <CreatorConfirm :open="gAsk" :title="Number(g.points) > 0 ? '確認發點' : '確認扣點'" :text="`${Number(g.points) > 0 ? '發放' : '扣除'} ${fmtPts(Math.abs(Number(g.points)))} 點給 ${target?.display_name}，原因：${g.reason}`" :ok="Number(g.points) > 0 ? '確認發放' : '確認扣除'" :danger="Number(g.points) < 0" :busy="gBusy" @close="gAsk = false" @ok="grant" />
    <CreatorConfirm :open="!!fz" :title="fz?.status === 'frozen' ? '解凍錢包' : '凍結錢包'" :text="`${fz?.display_name}（${fz?.email}）`" :ok="fz?.status === 'frozen' ? '解凍' : '凍結'" :danger="fz?.status !== 'frozen'" :busy="fzBusy" :disabled="!fzReason.trim()" :error="fzErr" @close="fz = null" @ok="setStatus">
      <label class="c-field"><span>原因（必填）</span><input v-model="fzReason" maxlength="200"></label>
      <p v-if="fz?.status !== 'frozen'" class="c-mute c-small">凍結後該使用者無法再贊助；既有認捐不受影響。</p>
    </CreatorConfirm>
  </main>
</template>
