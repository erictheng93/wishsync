<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
useHead({ title: '好友' })
const { api } = useApi()
const data = ref<any>({ friends: [], incoming: [], outgoing: [] })
const loading = ref(true), err = ref(''), msg = ref('')
const to = ref(''), sending = ref(false)
const invite = ref<any>(null), copied = ref(false)
const unfriend = ref<any>(null), ufBusy = ref(false), ufErr = ref('')

const msgOf = (e: any) => apiErrMsg(e)
async function load() {
  try { data.value = await api('/friends') } catch (e) { err.value = msgOf(e) } finally { loading.value = false }
}
onMounted(load)
// 動作後重新載入清單；失敗顯示錯誤
async function act(f: () => Promise<any>) {
  err.value = msg.value = ''
  try { await f(); await load() } catch (e) { err.value = msgOf(e) }
}
async function send() {
  const v = to.value.trim().replace(/^@/, '')
  if (!v) return
  sending.value = true; err.value = msg.value = ''
  try {
    await api('/friends/requests', { method: 'POST', body: { to: v } })
    msg.value = '若對方有帳號，已送出邀請'; to.value = ''; await load()
  } catch (e) { err.value = msgOf(e) } finally { sending.value = false }
}
const accept = (r: any) => act(() => api(`/friends/requests/${r.id}/accept`, { method: 'POST' }))
const dropReq = (r: any) => act(() => api(`/friends/requests/${r.id}`, { method: 'DELETE' }))
async function doUnfriend() {
  ufBusy.value = true; ufErr.value = ''
  try { await api(`/friends/${unfriend.value.id}`, { method: 'DELETE' }); unfriend.value = null; await load() }
  catch (e) { ufErr.value = msgOf(e) } finally { ufBusy.value = false }
}
const makeInvite = () => act(async () => { invite.value = await api('/friends/invites', { method: 'POST' }); copied.value = false })
const revoke = () => act(async () => { await api('/friends/invites', { method: 'DELETE' }); invite.value = null })
async function copy() {
  try { await navigator.clipboard.writeText(invite.value.url); copied.value = true } catch { err.value = '無法複製，請手動選取網址' }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="好友" back="/dashboard" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">載入中…</p>
    <template v-else>
      <section class="c-card">
        <h2 class="c-h2">加好友</h2>
        <form @submit.prevent="send">
          <label class="c-field"><span>對方的 Email 或帳號代號</span><input v-model="to" maxlength="120" required placeholder="name@example.com 或 @handle"></label>
          <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
          <button class="c-btn primary" :disabled="sending">{{ sending ? '送出中…' : '送出邀請' }}</button>
        </form>
      </section>

      <section class="c-card">
        <h2 class="c-h2">邀請連結</h2>
        <p class="c-mute">把連結或 QR 給朋友，對方登入後即可成為好友。連結 7 天有效，重新產生會讓舊連結失效。</p>
        <template v-if="invite">
          <label class="c-field"><span>邀請網址</span><input :value="invite.url" readonly @focus="($event.target as HTMLInputElement).select()"></label>
          <CreatorQr :text="invite.url" />
          <div class="c-row wrap c-mt">
            <button class="c-btn" @click="copy">{{ copied ? '已複製' : '複製網址' }}</button>
            <button class="c-btn" @click="makeInvite">重新產生</button>
            <button class="c-btn danger" @click="revoke">撤銷</button>
          </div>
        </template>
        <template v-else>
          <button class="c-btn primary" @click="makeInvite">產生邀請連結</button>
          <button class="c-btn danger" @click="revoke">撤銷既有邀請</button>
        </template>
      </section>

      <section v-if="data.incoming.length" class="c-card">
        <h2 class="c-h2">收到的邀請</h2>
        <div v-for="r in data.incoming" :key="r.id" class="c-row wrap c-mb">
          <span class="c-grow"><strong>{{ r.user.display_name }}</strong><small v-if="r.user.handle" class="c-mute"> @{{ r.user.handle }}</small></span>
          <button class="c-btn primary" @click="accept(r)">接受</button>
          <button class="c-btn" @click="dropReq(r)">婉拒</button>
        </div>
      </section>

      <section v-if="data.outgoing.length" class="c-card">
        <h2 class="c-h2">已送出的邀請</h2>
        <div v-for="r in data.outgoing" :key="r.id" class="c-row wrap c-mb">
          <span class="c-grow"><strong>{{ r.user.display_name }}</strong><small v-if="r.user.handle" class="c-mute"> @{{ r.user.handle }}</small></span>
          <button class="c-btn" @click="dropReq(r)">取消邀請</button>
        </div>
      </section>

      <section class="c-card">
        <h2 class="c-h2">我的好友（{{ data.friends.length }}）</h2>
        <p v-if="!data.friends.length" class="c-mute">還沒有好友，用上面的方式邀請朋友吧。</p>
        <div v-for="f in data.friends" :key="f.id" class="c-row wrap c-mb">
          <span class="c-grow">
            <NuxtLink v-if="f.handle" :to="`/u/${f.handle}`"><strong>{{ f.display_name }}</strong></NuxtLink>
            <strong v-else>{{ f.display_name }}</strong>
            <small v-if="f.handle" class="c-mute"> @{{ f.handle }}</small>
          </span>
          <button class="c-btn" @click="unfriend = f; ufErr = ''">解除好友</button>
        </div>
      </section>
    </template>
    <CreatorConfirm :open="!!unfriend" :title="`解除與 ${unfriend?.display_name} 的好友關係？`" text="對方將無法再看到你「僅好友」的清單，你在對方「指定名單」中的權限也會一併移除。" ok="解除好友" danger :busy="ufBusy" :error="ufErr" @close="unfriend = null" @ok="doUnfriend" />
  </main>
</template>
