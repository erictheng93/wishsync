<script setup lang="ts">
const props = defineProps<{ open: boolean; wishlistId: string; item: any | null }>()
const emit = defineEmits<{ close: []; saved: []; reload: []; address: [] }>()
const { api } = useApi()
const f = reactive<any>({})
const imageKey = ref<string | null>(null)
const version = ref<string | null>(null)
const upBusy = ref(false), busy = ref(false), err = ref(''), stale = ref(false), needAddr = ref(false)
// 已有認領 / 認捐的品項不能改取得方式；眾籌達標後不能改目標與期限（後端同樣會擋）
const modeLocked = computed(() => !!props.item && (props.item.qty_claimed > 0 || props.item.pledged_points > 0))
const fundLocked = computed(() => !!props.item && props.item.funding_mode === 'crowdfund' && props.item.funding_status !== 'open')
const crowd = computed(() => f.funding_mode === 'crowdfund')
watch(() => props.open, (v) => {
  if (!v) return
  const i = props.item
  Object.assign(f, { title: i?.title ?? '', brand: i?.brand ?? '', spec: i?.spec ?? '', product_url: i?.product_url ?? '', qty_needed: i?.qty_needed ?? 1, unit_price_amount: i?.unit_price_amount ?? '', priority: i?.priority ?? 'medium',
    funding_mode: i?.funding_mode === 'crowdfund' ? 'crowdfund' : 'quantity', target_points: i?.target_points ?? '', funding_deadline: toTaipeiInput(i?.funding_deadline), price_snapshot_amount: i?.price_snapshot_amount ?? i?.unit_price_amount ?? '' })
  imageKey.value = null; err.value = ''; stale.value = false; needAddr.value = false; version.value = i?.updated_at ?? null
})
async function save() {
  err.value = ''; needAddr.value = false
  if (!f.title.trim()) { err.value = '請輸入品項名稱'; return }
  const body: any = { title: f.title.trim(), brand: f.brand || null, spec: f.spec || null, product_url: f.product_url || null, priority: f.priority }
  if (crowd.value) {
    const t = Number(f.target_points)
    if (!Number.isInteger(t) || t < 1) { err.value = '請輸入目標點數（正整數）'; return }
    if (!fundLocked.value) {
      body.target_points = t
      if (f.funding_deadline) body.funding_deadline = fromTaipeiInput(f.funding_deadline) // 留空：後端用活動日 23:59（台北）
    }
    body.price_snapshot_amount = f.price_snapshot_amount === '' ? null : Number(f.price_snapshot_amount)
  } else {
    body.qty_needed = Number(f.qty_needed) || 1
    body.unit_price_amount = f.unit_price_amount === '' ? null : Number(f.unit_price_amount)
  }
  if (!props.item || f.funding_mode !== (props.item.funding_mode ?? 'quantity')) body.funding_mode = f.funding_mode
  if (imageKey.value) body.image_key = imageKey.value
  busy.value = true
  try {
    if (props.item) {
      if (version.value) body.expected_updated_at = version.value
      const r = await api(`/items/${props.item.id}`, { method: 'PATCH', body })
      version.value = r?.updated_at ?? version.value // 成功後更新本地版本
    }
    else await api(`/wishlists/${props.wishlistId}/items`, { method: 'POST', body })
    emit('saved')
  } catch (e: any) {
    if (e.code === 'STALE_VERSION') { stale.value = true; err.value = '這個品項已在其他地方被修改，請重新載入' }
    else if (e.code === 'SHIPPING_ADDRESS_REQUIRED') { needAddr.value = true; err.value = '眾籌品項需要先填寫收件資訊，達標後平台才知道要寄到哪裡。' }
    else err.value = errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <CreatorSheet :open="open" :title="item ? '編輯品項' : '新增品項'" @close="emit('close')">
    <form @submit.prevent="save">
      <CreatorImageUploader v-model="imageKey" purpose="item" :current-url="item?.image_url" @busy="upBusy = $event" />
      <label class="c-field"><span>名稱（必填）</span><input v-model="f.title" maxlength="120" placeholder="例如：奶瓶" required></label>
      <label class="c-field"><span>品牌（選填）</span><input v-model="f.brand" maxlength="100"></label>
      <label class="c-field"><span>規格（選填）</span><input v-model="f.spec" maxlength="200"></label>
      <label class="c-field"><span>商品網址（選填）</span><input v-model="f.product_url" type="url" maxlength="2000" placeholder="https://"></label>
      <div class="c-field"><span>取得方式</span>
        <div class="c-seg" style="margin:0"><label v-for="m in [['quantity', '數量認領'], ['crowdfund', '點數眾籌']]" :key="m[0]"><input v-model="f.funding_mode" type="radio" name="funding_mode" :value="m[0]" :disabled="modeLocked"><span>{{ m[1] }}</span></label></div>
        <p v-if="modeLocked" class="c-mute c-small">已有人認領或贊助，無法切換取得方式。</p>
      </div>
      <template v-if="crowd">
        <label class="c-field"><span>目標點數（必填，1 點 = NT$1）{{ item?.pledged_points ? `（已募 ${item.pledged_points}）` : '' }}</span><input v-model="f.target_points" type="number" inputmode="numeric" :min="(item?.pledged_points || 0) + 1" max="10000000" :disabled="fundLocked" required></label>
        <label class="c-field"><span>募集截止（台北時間，選填）</span><input v-model="f.funding_deadline" type="datetime-local" :disabled="fundLocked"><small class="c-mute">留空則使用活動日 23:59；清單沒有活動日時必須填寫。</small></label>
        <label class="c-field"><span>商品參考價格 NT$（選填）</span><input v-model="f.price_snapshot_amount" type="number" inputmode="numeric" min="0"></label>
        <p class="c-mute">達標後由平台代購寄出，點數不會轉成現金給你。需要先填寫清單的收件資訊。</p>
        <p v-if="fundLocked" class="c-mute">已達標或截止，目標與期限無法修改。</p>
      </template>
      <template v-else>
        <label class="c-field"><span>需要數量{{ item?.qty_claimed ? `（已被認領 ${item.qty_claimed}）` : '' }}</span><input v-model="f.qty_needed" type="number" inputmode="numeric" :min="item?.qty_claimed || 1" max="99"></label>
        <label class="c-field"><span>參考價格 NT$（選填）</span><input v-model="f.unit_price_amount" type="number" inputmode="numeric" min="0"></label>
      </template>
      <div class="c-field"><span>優先度</span>
        <div class="c-seg"><label v-for="p in [['high', '高'], ['medium', '中'], ['low', '低']]" :key="p[0]"><input v-model="f.priority" type="radio" :value="p[0]"><span>{{ p[1] }}</span></label></div>
      </div>
      <p v-if="err" class="c-err" role="alert">{{ err }} <button v-if="stale" type="button" class="c-btn" @click="emit('reload')">重新載入</button><button v-if="needAddr" type="button" class="c-btn" @click="emit('address')">填寫收件資訊</button></p>
      <div class="c-row">
        <button type="button" class="c-btn c-grow" @click="emit('close')">取消</button>
        <button class="c-btn primary c-grow" :disabled="busy || upBusy">{{ busy ? '儲存中…' : '儲存品項' }}</button>
      </div>
    </form>
  </CreatorSheet>
</template>
