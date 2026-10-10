<script setup lang="ts">
import '~/assets/guest.css'
// 認領 / 修改認領 bottom sheet（docs 03 §5.7）。Idempotency-Key 於開啟時產生，重試沿用。
const props = defineProps<{ item: any, mine: any | null, online: boolean, loggedInName?: string }>()
const emit = defineEmits<{ close: [], done: [r: any], stale: [remaining: number], switchEdit: [] }>()
const { api, newKey } = useGuest()

// 登入者：預設帶帳號的 default_claim_visibility（分享頁不會預先載入 /me，這裡自己抓）；載入前不送，由伺服器套預設
const { fetchMe } = useAuth()
const vis = ref<string | undefined>()
if (props.loggedInName) fetchMe().then(u => { vis.value ??= u?.default_claim_visibility })
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
const name0 = name.value, note0 = note.value
const dirty = computed(() => name.value !== name0 || note.value !== note0 || !!contact.value || !!email.value)

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
        method: 'POST', headers: { 'Idempotency-Key': key }, retryAsNewGuest: true,
        body: { qty: qty.value, display_name: props.loggedInName ? undefined : name.value.trim() || undefined, contact: contact.value || undefined, email: email.value || undefined, note: note.value || undefined, visibility: props.loggedInName ? vis.value : undefined },
      })
    }
    let persisted = true
    if (r.guest_token) persisted = setGuestToken(r.guest_token)
    // 防呆：新認領卻沒拿到 token、本機也沒有（且不是登入帳號）→ 之後管不到這份認領，成功頁要警告而非謊稱已存
    const lost = !editing && !r.guest_token && !getGuestToken() && !props.loggedInName
    if (name.value.trim()) setPref('ws_nickname', name.value.trim())
    emit('done', { ...r, persisted, lost, email: email.value, name: name.value.trim() })
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
      if (!Object.keys(errs.value).length) msg.value = e.detail // 欄位下已有錯誤就不在橫幅重複
    } else if (c === 'IDEMPOTENCY_CONFLICT') msg.value = '請求重複，請關閉後重新操作'
    else msg.value = e.detail
  } finally { busy.value = false }
}
</script>

<template>
  <GuestDialog labelledby="claim-title" :dirty="dirty" @close="emit('close')">
    <form class="g-sheet" @submit.prevent="submit">
      <div class="g-item">
        <img v-if="item.image_url" class="g-thumb" :src="item.image_url" alt="">
        <div class="g-body"><div id="claim-title" class="g-title">{{ editing ? '修改認領' : '認領' }}「{{ item.title }}」</div>
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
          <input id="cn" v-model="name" maxlength="30" :readonly="busy" :required="needName" placeholder="例如：阿明" autocomplete="nickname">
          <div v-if="errs.name || errs.display_name" class="g-field-err">{{ errs.name || errs.display_name }}</div>
        </template>
        <template v-if="loggedInName && vis">
          <label for="cv">這筆捐助誰看得到</label>
          <select id="cv" v-model="vis" :disabled="busy"><option value="public">公開</option><option value="friends">僅好友</option><option value="private">私人</option></select>
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
  </GuestDialog>
</template>
