<script setup lang="ts">
definePageMeta({ middleware: 'staff' })
useHead({ title: '採購單佇列' })
const { api } = useApi()
const status = ref('pending')
const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false)
const err = ref(''), msg = ref('')
const ACTION: Record<string, string> = { placed: '標記已下單', shipped: '標記已出貨', delivered: '標記已送達', failed: '標記失敗', cancelled: '取消採購' }
const actLabel = computed(() => (act.value ? ACTION[act.value.to] ?? '' : ''))

async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/admin/orders', { query: { status: status.value || undefined, limit: 20, cursor: more ? cursor.value : undefined } })
    rows.value = more ? [...rows.value, ...r.data] : r.data; cursor.value = r.next_cursor
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
watch(status, () => load())
onMounted(() => load())

// 狀態更新：依目前狀態只提供合法動作（orderActions），欄位依目標狀態顯示
const act = ref<{ row: any, to: string } | null>(null)
const form = reactive({ merchant_order_id: '', amount: '' as string | number, tracking_no: '', failure_reason: '' })
const busy = ref(false), aerr = ref('')
function open(row: any, to: string) {
  act.value = { row, to }; aerr.value = ''
  Object.assign(form, { merchant_order_id: row.merchant_order_id ?? '', amount: row.amount ?? row.target_points ?? '', tracking_no: row.tracking_no ?? '', failure_reason: '' })
}
async function submit() {
  const { row, to } = act.value!
  const p = buildOrderPatch(to, form, row.target_points ?? null)
  if (p.error) { aerr.value = p.error; return }
  busy.value = true; aerr.value = ''
  try {
    const r = await api(`/admin/orders/${row.id}`, { method: 'PATCH', body: p.body })
    msg.value = `「${row.item?.title}」已${label(ORDER_STATUS, r.status)}` + (r.refunded_points ? `，差額 ${fmtPts(r.refunded_points)} 點已退回捐贈者` : '')
    act.value = null; await load()
  } catch (e: any) {
    aerr.value = e.code === 'INVALID_STATE_TRANSITION' ? '狀態已被更新過，請重新整理後再操作' : errMsg(e)
    if (e.code === 'INVALID_STATE_TRANSITION') load()
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader title="採購單佇列" back="/admin" />
    <label class="c-field"><span>狀態</span>
      <select v-model="status"><option value="">全部</option><option v-for="(t, k) in ORDER_STATUS" :key="k" :value="k">{{ t }}</option></select></label>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">重試</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">載入中…</p>
    <p v-else-if="!rows.length && !err" class="c-center c-mute">沒有符合的採購單</p>
    <article v-for="o in rows" :key="o.id" class="c-card" :class="{ bad: o.status === 'failed' }">
      <div class="c-row"><strong class="c-grow">{{ o.item?.title }}</strong><span class="c-badge" :class="o.status">{{ label(ORDER_STATUS, o.status) }}</span></div>
      <div class="c-mute">清單：{{ o.wishlist?.title }}<template v-if="o.item?.product_url">・<a :href="o.item.product_url" target="_blank" rel="noopener nofollow">商品連結</a></template></div>
      <dl class="c-dl">
        <dt>目標點數</dt><dd>{{ fmtPts(o.target_points) }}</dd>
        <dt>實際金額</dt><dd>{{ o.status === 'pending' ? '尚未回報' : fmtPts(o.amount) }}</dd>
        <template v-if="o.refunded_points"><dt>退回點數</dt><dd>{{ fmtPts(o.refunded_points) }}（差額已退回捐贈者）</dd></template>
        <template v-if="o.merchant_order_id"><dt>訂單編號</dt><dd>{{ o.merchant_order_id }}</dd></template>
        <template v-if="o.tracking_no"><dt>物流單號</dt><dd>{{ o.tracking_no }}</dd></template>
        <template v-if="o.failure_reason"><dt>失敗原因</dt><dd>{{ o.failure_reason }}</dd></template>
        <template v-if="o.operator"><dt>承辦</dt><dd>{{ o.operator.display_name }}</dd></template>
        <dt>建立</dt><dd>{{ fmtTime(o.created_at) }}</dd>
        <template v-if="o.placed_at"><dt>下單</dt><dd>{{ fmtTime(o.placed_at) }}</dd></template>
        <template v-if="o.shipped_at"><dt>出貨</dt><dd>{{ fmtTime(o.shipped_at) }}</dd></template>
        <template v-if="o.delivered_at"><dt>送達</dt><dd>{{ fmtTime(o.delivered_at) }}</dd></template>
      </dl>
      <div v-if="o.shipping_address" class="c-sub">
        <strong class="c-small">收件資訊（讀取已寫入稽核紀錄）</strong>
        <dl class="c-dl"><dt>收件人</dt><dd>{{ o.shipping_address.recipient_name }}</dd><dt>電話</dt><dd>{{ o.shipping_address.phone }}</dd><dt>地址</dt><dd>{{ o.shipping_address.address }}</dd></dl>
      </div>
      <div v-if="orderActions(o.status).length" class="c-row wrap c-mt">
        <button v-for="to in orderActions(o.status)" :key="to" class="c-btn c-grow" :class="to === 'failed' || to === 'cancelled' ? 'danger' : 'primary'" @click="open(o, to)">{{ ACTION[to] }}</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">載入更多</button>

    <CreatorConfirm :open="!!act" :title="actLabel" :text="act?.row.item?.title ?? ''" :ok="actLabel" :danger="act?.to === 'failed' || act?.to === 'cancelled'" :busy="busy" :error="aerr" @close="act = null" @ok="submit">
      <template v-if="act?.to === 'placed'">
        <label class="c-field"><span>商家訂單編號（必填）</span><input v-model="form.merchant_order_id" maxlength="100"></label>
        <label class="c-field"><span>實際金額（必填，不可超過目標 {{ fmtPts(act.row.target_points) }}）</span><input v-model="form.amount" type="number" inputmode="numeric" min="1" :max="act.row.target_points"></label>
        <p class="c-mute c-small">低於目標的差額會依認捐比例自動退回捐贈者錢包。</p>
      </template>
      <label v-if="act?.to === 'shipped'" class="c-field"><span>物流單號（選填）</span><input v-model="form.tracking_no" maxlength="100"></label>
      <template v-if="act?.to === 'failed' || act?.to === 'cancelled'">
        <label class="c-field"><span>原因（必填）</span><input v-model="form.failure_reason" maxlength="200"></label>
        <p class="c-mute c-small">所有已扣用的點數會全額退回捐贈者，品項回到募集中（未過期）或已截止，並通知捐贈者。</p>
      </template>
      <p v-if="act?.to === 'delivered'" class="c-mute">確認包裹已送達？品項會標為已履行並通知捐贈者。</p>
    </CreatorConfirm>
  </main>
</template>
