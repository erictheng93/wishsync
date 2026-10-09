<script setup lang="ts">
import '~/assets/guest.css'
// 退訂認領通知（二步）：信件連結 → GET /unsubscribe 302 到這裡；必須由人按鈕才 POST，避免信件掃描器預取就誤退訂（F-26）。
// 舊流程 /unsubscribed?ok=1 仍保留（pages/unsubscribed.vue）。
useSeoMeta({ title: '退訂通知', robots: 'noindex' })
useHead({ meta: [{ name: 'referrer', content: 'no-referrer' }] }) // token 在網址上，不外洩
const token = String(useRoute().query.token ?? '')
const { api } = useGuest()
const state = ref<'ask' | 'busy' | 'done' | 'invalid' | 'error'>(token ? 'ask' : 'invalid')
const msg = ref('')
async function submit() {
  state.value = 'busy'
  try { await api('/unsubscribe', { method: 'POST', body: { token } }); state.value = 'done' }
  catch (e: any) {
    if (e.code === 'INVALID_TOKEN' || e.status === 422) state.value = 'invalid'
    else { msg.value = e.detail; state.value = 'error' }
  }
}
</script>

<template>
  <GuestTop />
  <main class="g-wrap g-center" style="padding-inline:16px">
    <template v-if="state === 'done'">
      <h1>已退訂</h1>
      <p class="g-mute">之後不會再寄「有人認領」的通知信。想重新開啟，登入後到「帳號設定」即可。</p>
      <NuxtLink to="/settings">前往帳號設定</NuxtLink>
    </template>
    <template v-else-if="state === 'invalid'">
      <h1>連結已失效</h1>
      <p class="g-mute">這個退訂連結無效或已過期，請登入後到「帳號設定」關閉通知。</p>
      <NuxtLink to="/settings">前往帳號設定</NuxtLink>
    </template>
    <template v-else>
      <h1>確定要退訂認領通知？</h1>
      <p class="g-mute">退訂後，有人認領你的清單時不會再寄信通知你。</p>
      <div v-if="state === 'error'" class="g-banner err" role="alert">{{ msg }}</div>
      <button class="g-btn" :disabled="state === 'busy'" @click="submit">{{ state === 'busy' ? '處理中…' : '確定退訂' }}</button>
    </template>
  </main>
</template>
