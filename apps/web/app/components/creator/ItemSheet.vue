<script setup lang="ts">
const props = defineProps<{ open: boolean; wishlistId: string; item: any | null }>()
const emit = defineEmits<{ close: []; saved: [] }>()
const { api } = useApi()
const f = reactive<any>({})
const imageKey = ref<string | null>(null)
const upBusy = ref(false), busy = ref(false), err = ref('')
watch(() => props.open, (v) => {
  if (!v) return
  const i = props.item
  Object.assign(f, { title: i?.title ?? '', brand: i?.brand ?? '', spec: i?.spec ?? '', product_url: i?.product_url ?? '', qty_needed: i?.qty_needed ?? 1, unit_price_amount: i?.unit_price_amount ?? '', priority: i?.priority ?? 'medium' })
  imageKey.value = null; err.value = ''
})
async function save() {
  err.value = ''
  if (!f.title.trim()) { err.value = '請輸入品項名稱'; return }
  const body: any = { title: f.title.trim(), brand: f.brand || null, spec: f.spec || null, product_url: f.product_url || null, qty_needed: Number(f.qty_needed) || 1, unit_price_amount: f.unit_price_amount === '' ? null : Number(f.unit_price_amount), priority: f.priority }
  if (imageKey.value) body.image_key = imageKey.value
  busy.value = true
  try {
    if (props.item) await api(`/items/${props.item.id}`, { method: 'PATCH', body })
    else await api(`/wishlists/${props.wishlistId}/items`, { method: 'POST', body: { ...body, funding_mode: 'quantity' } })
    emit('saved')
  } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
</script>
<template>
  <CreatorSheet :open="open" :title="item ? '編輯品項' : '新增品項'" @close="emit('close')">
    <form @submit.prevent="save">
      <CreatorImageUploader v-model="imageKey" purpose="item" :current-url="item?.image_url" @busy="upBusy = $event" />
      <label class="c-field"><span>名稱（必填）</span><input v-model="f.title" maxlength="80" placeholder="例如：奶瓶" required></label>
      <label class="c-field"><span>品牌（選填）</span><input v-model="f.brand"></label>
      <label class="c-field"><span>規格（選填）</span><input v-model="f.spec"></label>
      <label class="c-field"><span>商品網址（選填）</span><input v-model="f.product_url" type="url" placeholder="https://"></label>
      <label class="c-field"><span>需要數量{{ item?.qty_claimed ? `（已被認領 ${item.qty_claimed}）` : '' }}</span><input v-model="f.qty_needed" type="number" inputmode="numeric" :min="item?.qty_claimed || 1" max="99"></label>
      <label class="c-field"><span>參考價格 NT$（選填）</span><input v-model="f.unit_price_amount" type="number" inputmode="numeric" min="0"></label>
      <div class="c-field"><span>優先度</span>
        <div class="c-seg"><label v-for="p in [['high', '高'], ['medium', '中'], ['low', '低']]" :key="p[0]"><input v-model="f.priority" type="radio" :value="p[0]"><span>{{ p[1] }}</span></label></div>
      </div>
      <p class="c-mute">取得方式：數量認領（眾籌將於後續開放）</p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <div class="c-row">
        <button type="button" class="c-btn c-grow" @click="emit('close')">取消</button>
        <button class="c-btn primary c-grow" :disabled="busy || upBusy">{{ busy ? '儲存中…' : '儲存品項' }}</button>
      </div>
    </form>
  </CreatorSheet>
</template>
