<script setup lang="ts">
// 清單收件資訊（眾籌達標後平台代購寄送用）：只顯示遮罩後的值，要修改需重新輸入完整內容。
const props = defineProps<{ wishlistId: string }>()
const emit = defineEmits<{ saved: [r: any] }>()
const { api } = useApi()
const cur = ref<any>(null)
const f = reactive({ recipient_name: '', phone: '', address: '' })
const busy = ref(false), err = ref(''), msg = ref('')
const errs = ref<Record<string, string>>({})
onMounted(async () => {
  try { cur.value = await api(`/wishlists/${props.wishlistId}/shipping-address`); emit('saved', cur.value) } catch (e) { err.value = errMsg(e) }
})
async function save() {
  err.value = msg.value = ''; errs.value = {}
  busy.value = true
  try {
    cur.value = await api(`/wishlists/${props.wishlistId}/shipping-address`, { method: 'PUT', body: { recipient_name: f.recipient_name.trim(), phone: f.phone.trim(), address: f.address.trim() } })
    Object.assign(f, { recipient_name: '', phone: '', address: '' }) // 明文不留在畫面上
    msg.value = '已儲存'; emit('saved', cur.value)
  } catch (e: any) {
    for (const x of e.errors ?? []) errs.value[String(x.pointer ?? '').slice(1)] = x.detail
    err.value = Object.keys(errs.value).length ? '' : errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <div class="c-mt12">
    <p class="c-mute">點數眾籌達標後，由平台代購並寄到這裡。只有你與負責下單的營運人員看得到，朋友永遠看不到。</p>
    <dl v-if="cur?.has_shipping_address" class="c-dl"><dt>收件人</dt><dd>{{ cur.recipient_name }}</dd><dt>電話</dt><dd>{{ cur.phone }}</dd><dt>地址</dt><dd>{{ cur.address }}</dd></dl>
    <p v-else-if="cur" class="c-err">尚未填寫。含眾籌品項的清單必須先填寫才能發佈。</p>
    <form @submit.prevent="save">
      <label class="c-field"><span>收件人（1–50 字）</span><input v-model="f.recipient_name" maxlength="50" required autocomplete="name"><small v-if="errs.recipient_name" class="c-err">{{ errs.recipient_name }}</small></label>
      <label class="c-field"><span>電話（6–20 碼，可含 + - 與空白）</span><input v-model="f.phone" type="tel" maxlength="20" minlength="6" required autocomplete="tel"><small v-if="errs.phone" class="c-err">{{ errs.phone }}</small></label>
      <label class="c-field"><span>地址（5–200 字）</span><input v-model="f.address" maxlength="200" minlength="5" required autocomplete="street-address"><small v-if="errs.address" class="c-err">{{ errs.address }}</small></label>
      <p v-if="cur?.has_shipping_address" class="c-mute c-small">為了保護隱私，修改時需重新輸入完整內容。</p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
      <button class="c-btn block" :disabled="busy">{{ busy ? '儲存中…' : cur?.has_shipping_address ? '更新收件資訊' : '儲存收件資訊' }}</button>
    </form>
  </div>
</template>
