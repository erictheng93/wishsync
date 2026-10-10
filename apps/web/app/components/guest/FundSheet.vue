<script setup lang="ts">
import '~/assets/guest.css'
// 「用點數贊助」bottom sheet（契約 POST /items/{id}/contributions）。需要登入；Idempotency-Key 於開啟時產生，
// 確定性失敗後換新 key（body 可能已改），網路失敗才沿用同一把重試。
const props = defineProps<{ item: any, online: boolean, slug: string, loggedInName?: string }>()
const emit = defineEmits<{ close: [], done: [r: any], stale: [] }>()
const { api } = useApi()

const key = ref(crypto.randomUUID())
const remaining = computed(() => props.item.remaining_points ?? 0)
const points = ref<number | ''>(Math.min(remaining.value, 500) || remaining.value || '')
const message = ref(''), anon = ref(false)
const balance = ref<number | null>(null)
const busy = ref(false), msg = ref(''), needWallet = ref(false), needLogin = ref(false)
const dirty = computed(() => !!message.value || anon.value)
const chips = computed(() => quickAmounts(remaining.value))

onMounted(async () => { try { balance.value = (await api('/wallet', { noRedirect: true })).wallet.balance } catch { /* 餘額只是提示 */ } })

const valid = computed(() => Number.isInteger(points.value) && (points.value as number) >= 1 && (points.value as number) <= Math.min(10_000_000, remaining.value || 10_000_000))
const after = computed(() => balance.value != null && typeof points.value === 'number' ? balance.value - points.value : null)

async function submit() {
  msg.value = ''; needWallet.value = needLogin.value = false
  if (!valid.value) { msg.value = `請輸入 1 到 ${fmtPts(remaining.value)} 之間的整數點數`; return }
  busy.value = true
  try {
    const r = await api(`/items/${props.item.id}/contributions`, {
      method: 'POST', headers: { 'Idempotency-Key': key.value }, noRedirect: true,
      body: { points: points.value, message: message.value.trim() || undefined, is_anonymous: anon.value },
    })
    emit('done', r)
  } catch (e: any) {
    if (e.code === 'UNAUTHORIZED') { needLogin.value = true; msg.value = '登入已失效，請重新登入後再贊助' } else {
      const c = contributionErr(e.code, e.body, errMsg(e))
      msg.value = c.msg; needWallet.value = !!c.wallet
      if (c.points) points.value = c.points
      if (e.body?.balance != null) balance.value = e.body.balance
      if (['ITEM_FUNDED', 'FUNDING_EXPIRED', 'CROWDFUND_TARGET_EXCEEDED'].includes(e.code)) emit('stale')
    }
    if (e.code !== 'NETWORK') key.value = crypto.randomUUID()
  } finally { busy.value = false }
}
</script>

<template>
  <GuestDialog labelledby="fund-title" :dirty="dirty" @close="emit('close')">
    <form class="g-sheet" @submit.prevent="submit">
      <div class="g-item">
        <img v-if="item.image_url" class="g-thumb" :src="item.image_url" alt="">
        <div class="g-body"><div id="fund-title" class="g-title">用點數贊助「{{ item.title }}」</div>
          <div class="g-mute">還差 {{ fmtPts(remaining) }} 點達標<template v-if="item.funding_deadline">・募集至 {{ fmtTime(item.funding_deadline) }}</template></div></div>
      </div>
      <p class="g-mute">以 {{ loggedInName || '你的帳號' }} 的身分贊助。<template v-if="balance != null">目前餘額 <b class="g-num">{{ fmtPts(balance) }}</b> 點。</template>達標前可在「我的點數」撤回。</p>
      <label for="fp">贊助點數</label>
      <input id="fp" v-model.number="points" type="number" inputmode="numeric" min="1" :max="remaining || undefined" step="1" :readonly="busy" required>
      <div class="g-chips">
        <button v-for="n in chips" :key="n" type="button" class="g-btn ghost sm" :class="{ on: points === n }" :disabled="busy" @click="points = n">{{ fmtPts(n) }}</button>
        <button type="button" class="g-btn ghost sm" :class="{ on: points === remaining }" :disabled="busy || !remaining" @click="points = remaining">剩餘全額 {{ fmtPts(remaining) }}</button>
      </div>
      <p v-if="after != null && after < 0" class="g-field-err">餘額不足，還缺 {{ fmtPts(-after) }} 點</p>
      <label for="fm">留言（選填）</label>
      <input id="fm" v-model="message" maxlength="200" :readonly="busy">
      <label class="g-radio"><input v-model="anon" type="checkbox" :disabled="busy">匿名贊助（頁面上顯示為「匿名朋友」）</label>
      <p class="g-mute">點數用於平台代購，達標後由平台下單寄給壽星，不會轉成現金給建立者。</p>
      <div v-if="msg" class="g-banner err" role="alert">{{ msg }}
        <NuxtLink v-if="needWallet" to="/me/wallet">查看我的點數</NuxtLink>
        <NuxtLink v-if="needLogin" :to="{ path: '/login', query: { redirect: `/s/${slug}` } }">重新登入</NuxtLink>
      </div>
      <button class="g-btn" :disabled="busy || !online">{{ !online ? '離線中，暫時無法贊助' : busy ? '送出中…' : `確認贊助 ${valid ? fmtPts(points as number) + ' 點' : ''}` }}</button>
      <button type="button" class="g-link" @click="emit('close')">取消</button>
    </form>
  </GuestDialog>
</template>
