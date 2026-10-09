<script setup lang="ts">
import '~/assets/guest.css'
const props = defineProps<{ slug: string }>()
const emit = defineEmits<{ close: [], sent: [] }>()
const { api } = useGuest()
const reasons = [['scam', '疑似詐騙'], ['inappropriate', '不當內容'], ['copyright', '侵害著作權'], ['personal_info', '洩漏個人資料'], ['other', '其他']]
const reason = ref('scam'), detail = ref(''), busy = ref(false), msg = ref('')
// Turnstile：有設定 runtimeConfig.public.turnstileSiteKey 才載入 widget；否則送 'dev-bypass'（僅供本機 mock，後端需對應放行）
const siteKey = useRuntimeConfig().public.turnstileSiteKey as string
const token = ref(siteKey ? '' : 'dev-bypass')
const box = ref<HTMLElement>()
let wid: string | undefined
const SRC = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit'
function loadTurnstile(): Promise<any> {
  const w = window as any
  if (w.turnstile) return Promise.resolve(w.turnstile)
  return new Promise((ok, no) => {
    const s = document.querySelector<HTMLScriptElement>(`script[src="${SRC}"]`) ?? Object.assign(document.createElement('script'), { src: SRC, async: true })
    s.addEventListener('load', () => ok(w.turnstile)); s.addEventListener('error', () => { s.remove(); no(new Error('load')) })
    if (!s.isConnected) document.head.append(s)
  })
}
function reset() { token.value = ''; if (wid !== undefined) (window as any).turnstile?.reset(wid) }
onMounted(async () => {
  if (!siteKey) return
  try {
    const t = await loadTurnstile()
    if (!box.value) return // 載入期間 sheet 已關閉
    wid = t.render(box.value, {
      sitekey: siteKey, theme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light', language: 'zh-tw',
      callback: (v: string) => { token.value = v; msg.value = '' },
      'expired-callback': () => { token.value = ''; msg.value = '驗證已過期，請重新驗證' },
      'timeout-callback': () => { token.value = ''; msg.value = '驗證逾時，請重新驗證' },
      'error-callback': () => { token.value = ''; msg.value = '人機驗證發生錯誤，請重新驗證或稍後再試' },
    })
  } catch { msg.value = '無法載入人機驗證，請檢查網路後重新開啟' }
})
onBeforeUnmount(() => { if (wid !== undefined) (window as any).turnstile?.remove(wid); wid = undefined })
async function submit() {
  if (!token.value) { msg.value = '請先完成驗證'; return }
  busy.value = true; msg.value = ''
  try {
    await api(`/public/wishlists/${props.slug}/reports`, { method: 'POST', body: { reason: reason.value, detail: detail.value || null, item_id: null, turnstile_token: token.value } })
    emit('sent')
  } catch (e: any) { const r = reportErrMsg(e); msg.value = r.msg; if (r.resetCaptcha && siteKey) reset() } finally { busy.value = false }
}
</script>
<template>
  <GuestDialog labelledby="report-title" :dirty="!!detail" @close="emit('close')">
    <form class="g-sheet" @submit.prevent="submit">
      <div id="report-title" class="g-title">檢舉此清單</div>
      <fieldset class="g-fieldset"><legend class="g-mute">檢舉原因</legend>
        <label v-for="[v, t] in reasons" :key="v" class="g-radio">
          <input v-model="reason" type="radio" :value="v"> {{ t }}</label>
      </fieldset>
      <label for="rd">補充說明（選填）</label>
      <textarea id="rd" v-model="detail" rows="3" maxlength="500" placeholder="請描述你看到的問題" />
      <div v-if="siteKey" ref="box" style="margin:12px 0;min-height:65px" />
      <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
      <button class="g-btn" :disabled="busy || !token">{{ busy ? '送出中…' : '送出檢舉' }}</button>
      <p class="g-mute">不需登入。我們會由人工審核，不會告知建立者是誰檢舉。</p>
      <button type="button" class="g-link" @click="emit('close')">取消</button>
    </form>
  </GuestDialog>
</template>
