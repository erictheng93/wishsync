<script setup lang="ts">
import '~/assets/guest.css'
const props = defineProps<{ slug: string }>()
const emit = defineEmits<{ close: [], sent: [] }>()
const { api } = useGuest()
const reasons = [['scam', '疑似詐騙'], ['inappropriate', '不當內容'], ['copyright', '侵害著作權'], ['personal_info', '洩漏個人資料'], ['other', '其他']]
const reason = ref('scam'), detail = ref(''), busy = ref(false), msg = ref('')
// Turnstile：有設定 runtimeConfig.public.turnstileSiteKey 才載入 widget；否則送 'dev-bypass'（僅供本機 mock）
const siteKey = (useRuntimeConfig().public as any).turnstileSiteKey as string | undefined
const token = ref(siteKey ? '' : 'dev-bypass')
const box = ref<HTMLElement>()
onMounted(() => {
  if (!siteKey) return
  const go = () => (window as any).turnstile.render(box.value, { sitekey: siteKey, callback: (t: string) => (token.value = t) })
  if ((window as any).turnstile) return go()
  const s = document.createElement('script'); s.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit'; s.onload = go; document.head.append(s)
})
async function submit() {
  if (!token.value) { msg.value = '請先完成驗證'; return }
  busy.value = true; msg.value = ''
  try {
    await api(`/public/wishlists/${props.slug}/reports`, { method: 'POST', body: { reason: reason.value, detail: detail.value || null, item_id: null, turnstile_token: token.value } })
    emit('sent')
  } catch (e: any) { msg.value = e.code === 'RATE_LIMITED' ? '檢舉太頻繁，請稍後再試' : e.detail } finally { busy.value = false }
}
</script>
<template>
  <div class="g-mask" @click.self="emit('close')">
    <form class="g-sheet" role="dialog" aria-modal="true" @submit.prevent="submit">
      <div class="g-title">檢舉此清單</div>
      <fieldset class="g-fieldset"><legend class="g-mute">檢舉原因</legend>
        <label v-for="[v, t] in reasons" :key="v" class="g-radio">
          <input v-model="reason" type="radio" :value="v"> {{ t }}</label>
      </fieldset>
      <label for="rd">補充說明（選填）</label>
      <textarea id="rd" v-model="detail" rows="3" maxlength="500" placeholder="請描述你看到的問題" />
      <div ref="box" style="margin:12px 0" />
      <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
      <button class="g-btn" :disabled="busy">{{ busy ? '送出中…' : '送出檢舉' }}</button>
      <p class="g-mute">不需登入。我們會由人工審核，不會告知建立者是誰檢舉。</p>
      <button type="button" class="g-link" @click="emit('close')">取消</button>
    </form>
  </div>
</template>
