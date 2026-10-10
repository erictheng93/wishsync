<script setup lang="ts">
useHead({ title: '好友邀請', meta: [{ name: 'robots', content: 'noindex' }] })
const route = useRoute()
const token = String(route.params.token)
const { api } = useApi()
const { user, fetchMe } = useAuth()
const inviter = ref<any>(null), state = ref<'loading' | 'ok' | 'gone' | 'error'>('loading')
const busy = ref(false), err = ref('')
onMounted(async () => {
  await fetchMe()
  try { inviter.value = (await api(`/friends/invites/${encodeURIComponent(token)}`, { noRedirect: true })).inviter; state.value = 'ok' }
  catch (e: any) { state.value = e.status === 404 ? 'gone' : 'error' }
})
async function accept() {
  busy.value = true; err.value = ''
  try { await api(`/friends/invites/${encodeURIComponent(token)}/accept`, { method: 'POST' }); await navigateTo('/friends') }
  catch (e: any) { err.value = e.status === 404 ? '這個邀請已失效或被撤銷' : apiErrMsg(e) } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="好友邀請" />
    <p v-if="state === 'loading'" class="c-mute" role="status">載入中…</p>
    <div v-else-if="state === 'gone'" class="c-center"><p>這個邀請連結已失效或被撤銷。</p><p class="c-mute">請向朋友索取新的連結。</p></div>
    <div v-else-if="state === 'error'" class="c-center"><p class="c-err" role="alert">讀取失敗，請稍後再試</p></div>
    <section v-else class="c-card">
      <h2 class="c-h2">{{ inviter.display_name }} 邀請你成為好友</h2>
      <p v-if="inviter.handle" class="c-mute">@{{ inviter.handle }}</p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button v-if="user" class="c-btn primary" :disabled="busy" @click="accept">{{ busy ? '處理中…' : '成為好友' }}</button>
      <NuxtLink v-else :to="{ path: '/login', query: { redirect: route.fullPath } }" class="c-btn primary">登入後成為好友</NuxtLink>
    </section>
  </main>
</template>
