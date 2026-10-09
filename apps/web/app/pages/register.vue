<script setup lang="ts">
useHead({ title: '建立帳號' })
const route = useRoute()
const { register, verifyRegister } = useAuth()
const raw = String(route.query.redirect || '')
const redirect = raw.startsWith('/') && !raw.startsWith('//') ? raw : '/dashboard'
const step = ref<'form' | 'otp'>('form')
const email = ref(''), password = ref(''), name = ref(''), code = ref('')
const otp = ref<{ focus: () => void }>()
const busy = ref(false), err = ref(''), wait = ref(0)
let t: any
function tick(n: number) { wait.value = n; clearInterval(t); t = setInterval(() => { if (--wait.value <= 0) clearInterval(t) }, 1000) }
onBeforeUnmount(() => clearInterval(t))

async function send() {
  err.value = ''
  if (password.value.length < 8) { err.value = '密碼至少需 8 個字元'; return }
  busy.value = true
  try {
    const r = await register(email.value.trim(), password.value, name.value.trim())
    step.value = 'otp'; code.value = ''; tick(r.resend_after || 60)
    nextTick(() => otp.value?.focus())
  } catch (e: any) { err.value = e.status === 429 ? '請求過於頻繁，請稍後再試' : errMsg(e) } finally { busy.value = false }
}
async function verify() {
  if (code.value.length < 6 || busy.value) return
  err.value = ''; busy.value = true
  try {
    await verifyRegister(email.value.trim(), code.value)
    await navigateTo(redirect)
  } catch (e: any) {
    if (e.code === 'EMAIL_EXISTS') err.value = '此 Email 已註冊，請直接登入或使用忘記密碼'
    else { err.value = e.code === 'OTP_INVALID' ? '驗證碼不正確或已過期，請再試一次' : e.status === 429 ? '嘗試次數過多，請稍後再試' : errMsg(e); code.value = ''; otp.value?.focus() }
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="建立帳號" back="/login" />
    <form v-if="step === 'form'" @submit.prevent="send">
      <label class="c-field"><span>顯示名稱</span><input v-model="name" type="text" autocomplete="nickname" maxlength="50" required></label>
      <label class="c-field"><span>Email</span><input v-model="email" type="email" autocomplete="email" placeholder="name@example.com" required></label>
      <CreatorPasswordField v-model="password" label="密碼" autocomplete="new-password" hint="至少 8 個字元，最多 128 個字元" />
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block c-mb" :disabled="busy">{{ busy ? '寄送中…' : '寄送驗證碼' }}</button>
      <p class="c-center c-mute">已有帳號？<NuxtLink :to="{ path: '/login', query: route.query.redirect ? { redirect } : {} }">登入</NuxtLink></p>
    </form>
    <template v-else>
      <p>驗證碼已寄到 {{ email }}</p>
      <CreatorOtpInput ref="otp" v-model="code" @complete="verify" />
      <p v-if="err" class="c-err c-center" role="alert">{{ err }}</p>
      <NuxtLink v-if="err.includes('已註冊')" class="c-btn block c-mb" to="/login">前往登入</NuxtLink>
      <button class="c-btn primary block c-mb" :disabled="busy || code.length < 6" @click="verify">{{ busy ? '驗證中…' : '完成註冊' }}</button>
      <p class="c-center c-mute">
        <button v-if="wait <= 0" class="c-btn" :disabled="busy" @click="send">重新寄送</button>
        <span v-else>{{ wait }} 秒後可重新寄送</span>
      </p>
      <button class="c-btn block" @click="step = 'form'; err = ''">修改資料</button>
    </template>
  </main>
</template>
