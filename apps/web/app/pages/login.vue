<script setup lang="ts">
useHead({ title: '登入' })
const route = useRoute()
const { base } = useApi()
const { fetchMe, login } = useAuth()
const raw = String(route.query.redirect || '')
const redirect = raw.startsWith('/') && !raw.startsWith('//') ? raw : '/dashboard'
const email = ref(''), password = ref('')
const busy = ref(false)
const err = ref(route.query.error === 'oauth_failed' ? '第三方登入失敗，請重試' : route.query.error === 'oauth_unavailable' ? '此登入方式尚未開放' : '')
const okMsg = route.query.reset ? '密碼已重設，請用新密碼登入' : ''
const q = redirect === '/dashboard' ? {} : { redirect }
onMounted(async () => { if (await fetchMe()) navigateTo(redirect) })

function oauth(p: 'line' | 'google') {
  window.location.href = `${base}/auth/oauth/${p}/start?redirect=${encodeURIComponent(redirect)}`
}
async function submit() {
  err.value = ''; busy.value = true
  try {
    await login(email.value.trim(), password.value)
    await navigateTo(redirect)
  } catch (e: any) {
    err.value = e.code === 'INVALID_CREDENTIALS' ? '帳號或密碼錯誤' : e.status === 429 ? '嘗試次數過多，請稍後再試' : errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="登入" back="/" />
    <p v-if="okMsg" class="c-ok" role="status">{{ okMsg }}</p>
    <form @submit.prevent="submit">
      <label class="c-field"><span>Email</span><input v-model="email" type="email" autocomplete="email" placeholder="name@example.com" required></label>
      <CreatorPasswordField v-model="password" label="密碼" autocomplete="current-password" />
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block c-mb" :disabled="busy">{{ busy ? '登入中…' : '登入' }}</button>
    </form>
    <div class="c-links">
      <NuxtLink :to="{ path: '/register', query: q }">建立帳號</NuxtLink>
      <NuxtLink to="/forgot-password">忘記密碼</NuxtLink>
    </div>
    <p class="c-center c-mute">或</p>
    <button type="button" class="c-btn block c-mb" @click="oauth('google')">使用 Google 繼續</button>
    <button type="button" class="c-btn block c-mb" @click="oauth('line')">使用 LINE 繼續</button>
    <p class="c-mute">送禮的朋友不需要登入，直接用分享連結即可。</p>
  </main>
</template>
