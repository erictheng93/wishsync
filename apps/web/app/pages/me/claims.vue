<script setup lang="ts">
import '~/assets/guest.css'
useSeoMeta({ title: '我的認領', robots: 'noindex' })
const { api } = useGuest()
const state = ref<'loading' | 'ok' | 'empty' | 'error'>('loading')
const guest = ref<any>(null), rows = ref<any[]>([]), recoverFailed = ref(false), msg = ref('')
const editing = ref(false), nick = ref('')
const label: Record<string, string> = { reserved: '已認領', purchased: '已購買', delivered: '已送達', cancelled: '已取消', expired: '已逾期' }
const active = computed(() => rows.value.filter(r => ['reserved', 'purchased', 'delivered'].includes(r.claim.status)))
const past = computed(() => rows.value.filter(r => !active.value.includes(r)))
const groups = computed(() => {
  const m = new Map<string, any>()
  for (const r of active.value) (m.get(r.wishlist.slug) ?? m.set(r.wishlist.slug, { w: r.wishlist, list: [] }).get(r.wishlist.slug)).list.push(r)
  return [...m.values()]
})

async function load() {
  state.value = 'loading'
  try {
    const r = await api('/guest/me?limit=100')
    guest.value = r.guest; rows.value = r.claims; nick.value = r.guest.display_name
    state.value = rows.value.length ? 'ok' : 'empty'
  } catch (e: any) {
    if (e.status === 401) { setGuestToken(null); state.value = 'empty' } else state.value = 'error'
  }
}
onMounted(async () => {
  const m = location.hash.match(/^#r=(.+)$/)
  if (m) {
    history.replaceState(null, '', location.pathname) // 一次性權杖，立即從網址移除
    try { setGuestToken((await api('/guest/recover', { method: 'POST', body: { token: m[1] } })).guest_token) } catch { recoverFailed.value = true }
  }
  load()
})

const guard = async (f: () => Promise<any>) => { msg.value = ''; try { await f() } catch (e: any) { msg.value = e.detail } }
const setStatus = (r: any, status: string) => guard(async () => { Object.assign(r.claim, (await api(`/claims/${r.claim.id}`, { method: 'PATCH', body: { status } })).claim) })
const cancel = (r: any) => {
  const t = r.claim.status === 'purchased' ? '建立者會收到通知，確定要取消認領？' : '確定要取消這筆認領？'
  if (confirm(t)) guard(async () => { await api(`/claims/${r.claim.id}`, { method: 'DELETE' }); r.claim.status = 'cancelled' })
}
const saveNick = () => guard(async () => { guest.value = (await api('/guest/me', { method: 'PATCH', body: { display_name: nick.value } })).guest; editing.value = false })
const wipe = () => {
  if (confirm('確定刪除你的暱稱與聯絡方式？認領紀錄會保留為「已刪除的訪客」，且此裝置之後無法再管理。')) guard(async () => {
    await api('/guest/me', { method: 'DELETE' }); setGuestToken(null); guest.value = null; rows.value = []; state.value = 'empty'
  })
}
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <h1>我的認領</h1>
    <div v-if="recoverFailed" class="g-banner err" role="alert">這個管理連結已失效（只能使用一次，且 30 天內有效）。請回到朋友分享的清單連結，或聯絡客服。</div>
    <div v-if="msg" class="g-banner err" role="alert">{{ msg }}</div>
    <p v-if="state === 'loading'" class="g-mute">載入中…</p>
    <div v-else-if="state === 'error'" class="g-center"><p>讀取失敗</p><button class="g-btn" @click="load">重試</button></div>
    <div v-else-if="state === 'empty'" class="g-center">
      <p>你還沒有認領任何東西。收到朋友的清單連結後，就能在這裡管理。</p>
      <p class="g-mute">看不到之前的認領？可能是在不同瀏覽器或 LINE 內建瀏覽器。留過 Email 的認領，可從確認信的「管理我的認領」連結回來。</p>
    </div>
    <template v-else>
      <div class="g-card" v-if="guest">
        <template v-if="!editing"><b>{{ guest.display_name }}</b>{{ guest.is_user ? '（已登入）' : '（此裝置）' }}<button v-if="!guest.is_user" class="g-link" @click="editing = true">改暱稱</button></template>
        <form v-else class="g-inline" @submit.prevent="saveNick"><input v-model="nick" maxlength="30" required aria-label="暱稱"><button class="g-btn sm">儲存</button></form>
      </div>
      <section v-for="g in groups" :key="g.w.slug">
        <h2 class="g-h2"><NuxtLink :to="`/s/${g.w.slug}`">{{ g.w.title }}</NuxtLink></h2>
        <div v-for="r in g.list" :key="r.claim.id" class="g-card g-item">
          <img v-if="r.item.image_url" class="g-thumb" :src="r.item.image_url" alt=""><div v-else class="g-thumb">圖</div>
          <div class="g-body">
            <div class="g-title">{{ r.item.title }} × {{ r.claim.qty }}</div>
            <span class="g-badge">{{ label[r.claim.status] }}</span>
            <div class="g-actions">
              <button v-if="r.claim.status === 'reserved'" class="g-btn sm" @click="setStatus(r, 'purchased')">標記已購買</button>
              <button v-if="r.claim.status === 'purchased'" class="g-btn sm" @click="setStatus(r, 'delivered')">標記已送達</button>
              <button v-if="r.claim.status !== 'delivered'" class="g-btn ghost sm" @click="cancel(r)">取消</button>
            </div>
          </div>
        </div>
      </section>
      <details v-if="past.length"><summary class="g-mute">過去的紀錄（{{ past.length }}）</summary>
        <div v-for="r in past" :key="r.claim.id" class="g-mute">{{ r.item.title }} × {{ r.claim.qty }}・{{ label[r.claim.status] }}</div>
      </details>
      <div v-if="!guest?.is_user" class="g-foot"><button class="g-link" @click="wipe">刪除我的暱稱與聯絡方式</button></div>
    </template>
  </main>
</template>
