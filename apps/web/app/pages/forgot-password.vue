<script setup lang="ts">
useHead({ title: '忘記密碼' })
const { requestReset, confirmReset } = useAuth()
const step = ref<'email' | 'reset'>('email')
const email = ref(''), password = ref(''), code = ref('')
const otp = ref<{ focus: () => void }>()
const busy = ref(false), err = ref(''), wait = ref(0)
let t: any
function tick(n: number) { wait.value = n; clearInterval(t); t = setInterval(() => { if (--wait.value <= 0) clearInterval(t) }, 1000) }
onBeforeUnmount(() => clearInterval(t))

async function send() {
  err.value = ''; busy.value = true
  try {
    const r = await requestReset(email.value.trim())
    step.value = 'reset'; tick(r.resend_after || 60)
  } catch (e: any) { err.value = e.status === 429 ? '請求過於頻繁，請稍後再試' : errMsg(e) } finally { busy.value = false }
}
async function confirm() {
  err.value = ''
  if (code.value.length < 6) { err.value = '請輸入 6 位數驗證碼'; return }
  if (password.value.length < 8) { err.value = '新密碼至少需 8 個字元'; return }
  busy.value = true
  try {
    await confirmReset(email.value.trim(), code.value, password.value)
    await navigateTo({ path: '/login', query: { reset: '1' } })
  } catch (e: any) {
    err.value = e.code === 'OTP_INVALID' ? '驗證碼不正確或已過期，請再試一次' : e.status === 429 ? '嘗試次數過多，請稍後再試' : errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="忘記密碼" back="/login" />
    <form v-if="step === 'email'" @submit.prevent="send">
      <label class="c-field"><span>Email</span><input v-model="email" type="email" autocomplete="email" placeholder="name@example.com" required></label>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block" :disabled="busy">{{ busy ? '寄送中…' : '寄送驗證碼' }}</button>
    </form>
    <form v-else @submit.prevent="confirm">
      <p class="c-ok" role="status">若帳號存在，已寄出驗證碼</p>
      <span class="c-lbl">驗證碼</span>
      <CreatorOtpInput ref="otp" v-model="code" />
      <CreatorPasswordField v-model="password" label="新密碼" autocomplete="new-password" hint="至少 8 個字元，最多 128 個字元" />
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block c-mb" :disabled="busy">{{ busy ? '重設中…' : '重設密碼' }}</button>
      <p class="c-center c-mute">
        <button v-if="wait <= 0" type="button" class="c-btn" :disabled="busy" @click="send">重新寄送</button>
        <span v-else>{{ wait }} 秒後可重新寄送</span>
      </p>
    </form>
  </main>
</template>
