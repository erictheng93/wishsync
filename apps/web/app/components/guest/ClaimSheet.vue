<script setup lang="ts">
import '~/assets/guest.css'
// 認領 / 修改認領 bottom sheet（docs 03 §5.7）。Idempotency-Key 於開啟時產生，重試沿用。
const props = defineProps<{ item: any, mine: any | null, online: boolean, loggedInName?: string }>()
const emit = defineEmits<{ close: [], done: [r: any], stale: [remaining: number], switchEdit: [] }>()
const { api, newKey } = useGuest()

const key = newKey()
const editing = !!props.mine
const max = ref(props.item.qty_remaining + (props.mine?.qty ?? 0))
const qty = ref(Math.min(props.mine?.qty ?? 1, max.value))
const name = ref(getPref('ws_nickname') ?? '')
const contact = ref('')
const email = ref('') // 不從 localStorage 帶入
const note = ref(props.mine?.note ?? '')
const busy = ref(false)
const msg = ref('')
const errs = ref<Record<string, string>>({})
const needName = !getGuestToken() && !props.loggedInName

async function submit() {
  errs.value = {}; msg.value = ''
  if (needName && !name.value.trim()) { errs.value.name = '請填暱稱'; return }
  busy.value = true
  try {
    let r: any
    if (editing) {
      r = await api(`/claims/${props.mine.id}`, { method: 'PATCH', body: { qty: qty.value, note: note.value || null } })
    } else {
      r = await api(`/items/${props.item.id}/claims`, {
        method: 'POST', headers: { 'Idempotency-Key': key },
        body: { qty: qty.value, display_name: props.loggedInName ? undefined : name.value.trim() || undefined, contact: contact.value || undefined, email: email.value || undefined, note: note.value || undefined },
      })
    }
    let persisted = true
    if (r.guest_token) persisted = setGuestToken(r.guest_token)
    if (name.value.trim()) setPref('ws_nickname', name.value.trim())
    emit('done', { ...r, persisted, email: email.value, name: name.value.trim() })
  } catch (e: any) {
    const c = e.code
    if (c === 'ITEM_FULLY_CLAIMED') {
      const rem = (e.data.remaining ?? 0) + (props.mine?.qty ?? 0)
      emit('stale', e.data.remaining ?? 0)
      max.value = rem; qty.value = Math.min(qty.value, rem) || 1
      msg.value = rem > 0 ? `抱歉，只剩 ${rem} 個了。` : '剛剛被別人認領完了。'
      if (rem <= 0) setTimeout(() => emit('close'), 1200)
    } else if (c === 'CLAIM_ALREADY_EXISTS') {
      msg.value = '你已經認領過這個品項，已幫你切換到修改'
      emit('switchEdit')
    } else if (c === 'VALIDATION_FAILED') {
      for (const x of e.data.errors ?? []) errs.value[x.pointer.slice(1)] = x.detail
      msg.value = e.detail
    } else if (c === 'IDEMPOTENCY_CONFLICT') msg.value = '請求重複，請關閉後重新操作'
    else msg.value = e.status === 503 ? '系統維護中，暫時無法認領' : e.detail
  } finally { busy.value = false }
}
</script>

<template>
  <div class="g-mask" @click.self="emit('close')">
    <form class="g-sheet" role="dialog" aria-modal="true" @submit.prevent="submit">
      <div class="g-item">
        <img v-if="item.image_url" class="g-thumb" :src="item.image_url" alt="">
        <div class="g-body"><div class="g-title">{{ editing ? '修改認領' : '認領' }}「{{ item.title }}」</div>
          <div class="g-mute">還缺 {{ item.qty_remaining }} 個，你要送幾個？</div></div>
      </div>
      <div class="g-step">
        <button type="button" aria-label="減少" :disabled="busy || qty <= 1" @click="qty--">－</button>
        <b>{{ qty }}</b>
        <button type="button" aria-label="增加" :disabled="busy || qty >= max" @click="qty++">＋</button>
      </div>
      <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
      <template v-if="!editing">
        <p v-if="loggedInName" class="g-mute">以 {{ loggedInName }} 的身分認領</p>
        <template v-else>
          <label for="cn">你的暱稱（必填）</label>
          <input id="cn" v-model="name" maxlength="40" :readonly="busy" :required="needName" placeholder="例如：阿明" autocomplete="nickname">
          <div v-if="errs.name || errs.display_name" class="g-field-err">{{ errs.name || errs.display_name }}</div>
        </template>
        <label for="cc">聯絡方式（選填，只有建立者看得到）</label>
        <input id="cc" v-model="contact" maxlength="80" :readonly="busy" placeholder="LINE ID 或電話">
        <label for="ce">Email（選填）：可收到確認信與「管理我的認領」連結，換手機也找得回來</label>
        <input id="ce" v-model="email" type="email" :readonly="busy" placeholder="name@example.com" autocomplete="email">
        <div v-if="errs.email" class="g-field-err">{{ errs.email }}</div>
      </template>
      <label for="cm">留言（選填）</label>
      <input id="cm" v-model="note" maxlength="200" :readonly="busy">
      <p class="g-mute">不需註冊。認領後可隨時修改或取消。我們如何使用你的資料：<NuxtLink to="/privacy#collect" target="_blank">個資蒐集告知</NuxtLink>（不需勾選）</p>
      <button class="g-btn" :disabled="busy || !online">{{ !online ? '離線中，暫時無法認領' : busy ? '送出中…' : editing ? '儲存修改' : '確認認領' }}</button>
      <button type="button" class="g-link" @click="emit('close')">取消</button>
    </form>
  </div>
</template>
