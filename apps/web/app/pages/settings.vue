<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
useHead({ title: '帳號設定' })
const { api } = useApi()
const { user, fetchMe, logout } = useAuth()
const name = ref(''), msg = ref(''), err = ref('')
const exporting = ref(false)
const showDel = ref(false), word = ref(''), delBusy = ref(false), delErr = ref('')
onMounted(async () => { await fetchMe(true); name.value = user.value?.display_name ?? '' })

async function toggle(ev: Event) {
  const v = (ev.target as HTMLInputElement).checked, old = !v
  user.value.notification_prefs = { ...user.value.notification_prefs, email_claims: v } // 樂觀更新
  try { await api('/me', { method: 'PATCH', body: { notification_prefs: { email_claims: v } } }) }
  catch (e) { user.value.notification_prefs = { ...user.value.notification_prefs, email_claims: old }; err.value = errMsg(e) }
}
async function saveName() {
  msg.value = err.value = ''
  try { user.value = await api('/me', { method: 'PATCH', body: { display_name: name.value.trim() } }); msg.value = '已儲存' }
  catch (e) { err.value = errMsg(e) }
}
async function exportData() {
  exporting.value = true; err.value = ''
  try {
    const data = await api('/me/export')
    const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }))
    const a = Object.assign(document.createElement('a'), { href: url, download: `wishsync-export-${new Date().toISOString().slice(0, 10)}.json` })
    a.click(); URL.revokeObjectURL(url)
  } catch (e) { err.value = errMsg(e) } finally { exporting.value = false }
}
async function remove() {
  delBusy.value = true; delErr.value = ''
  try {
    await api('/me', { method: 'DELETE', body: { confirm: 'DELETE' } })
    user.value = null
    await navigateTo('/')
  } catch (e: any) {
    delErr.value = e.code === 'ACCOUNT_HAS_POINTS' ? `${e.detail}（可到「我的點數」撤回認捐）` : e.code === 'ACCOUNT_HAS_BALANCE' ? `${e.detail}（餘額 ${e.body.balance ?? 0} 點、未完成認捐 ${e.body.pledged_count ?? 0}、未結案訂單 ${e.body.open_order_count ?? 0}）請先處理後再刪除。` : errMsg(e)
  } finally { delBusy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="帳號設定" back="/dashboard" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <template v-if="user">
      <section class="c-card">
        <h2 class="c-h2">我的資料</h2>
        <p class="c-mute">{{ user.email }}</p>
        <form @submit.prevent="saveName">
          <label class="c-field"><span>顯示名稱</span><input v-model="name" maxlength="40" required></label>
          <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
          <button class="c-btn">儲存</button>
        </form>
      </section>
      <section class="c-card">
        <h2 class="c-h2">通知</h2>
        <label class="c-switch"><input type="checkbox" :checked="user.notification_prefs?.email_claims" @change="toggle"><span>有人認領時寄 Email<br><small class="c-mute">第一筆即時通知，之後每日彙整一封。驚喜模式下只寫「有 N 件新認領」。</small></span></label>
      </section>
      <section class="c-card">
        <h2 class="c-h2">匯出資料</h2>
        <p class="c-mute">包含個人資料、清單與認領紀錄（JSON）。</p>
        <button class="c-btn" :disabled="exporting" @click="exportData">{{ exporting ? '匯出中…' : '匯出我的資料' }}</button>
      </section>
      <section class="c-card">
        <button class="c-btn block" @click="logout">登出</button>
      </section>
      <section class="c-card bad">
        <h2 class="c-h2">刪除帳號</h2>
        <p class="c-mute">我們會移除你的 Email 與名稱，你的清單會封存並從公開移除。此動作無法復原。</p>
        <button class="c-btn danger" @click="showDel = true; word = ''; delErr = ''">刪除我的帳號</button>
      </section>
      <p class="c-mute"><NuxtLink to="/terms">服務條款</NuxtLink>・<NuxtLink to="/privacy">隱私權政策</NuxtLink></p>
    </template>
    <CreatorConfirm :open="showDel" title="確定要刪除帳號嗎？" text="你的 Email、名稱與登入方式會被移除，清單會封存並從公開移除，朋友無法再開啟。" ok="永久刪除" danger :busy="delBusy" :disabled="word !== '刪除'" :error="delErr" @close="showDel = false" @ok="remove">
      <label class="c-field"><span>請輸入「刪除」以確認</span><input v-model="word" autocomplete="off"></label>
    </CreatorConfirm>
  </main>
</template>
