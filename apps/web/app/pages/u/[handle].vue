<script setup lang="ts">
const route = useRoute()
const handle = String(route.params.handle)
const { api, base } = useApi()
const { user, fetchMe } = useAuth()
const p = ref<any>(null), state = ref<'loading' | 'ok' | 'gone' | 'error'>('loading')
const busy = ref(false), err = ref('')
const vis: Record<string, string> = { public: '公開', friends: '僅好友', selected: '指定好友', link: '連結' }
const status: Record<string, string> = { reserved: '已認領', purchased: '已購買', delivered: '已送達' }
useHead(() => ({ title: p.value ? `${p.value.user.display_name} 的心願` : '個人頁', meta: [{ name: 'robots', content: 'noindex' }] }))

// 匿名也要能看：不走 useApi 的 401 轉址，直接帶 cookie 呼叫
async function load() {
  try { p.value = await $fetch(`${base}/users/${encodeURIComponent(handle)}`, { credentials: 'include' }); state.value = 'ok' }
  catch (e: any) { state.value = e.status === 404 || e.statusCode === 404 ? 'gone' : 'error' }
}
onMounted(async () => { await fetchMe(); await load() })

async function run(f: () => Promise<any>) {
  busy.value = true; err.value = ''
  try { await f(); await load() } catch (e) { err.value = apiErrMsg(e as any) } finally { busy.value = false }
}
const add = () => run(() => api('/friends/requests', { method: 'POST', body: { to: handle } }))
// 個人頁沒給申請 id，從 /friends 找對方送來的那筆
const accept = () => run(async () => {
  const f = await api('/friends')
  const r = f.incoming.find((x: any) => x.user.id === p.value.user.id)
  if (r) await api(`/friends/requests/${r.id}/accept`, { method: 'POST' })
})
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="p ? p.user.display_name : '個人頁'" />
    <p v-if="state === 'loading'" class="c-mute" role="status">載入中…</p>
    <div v-else-if="state === 'gone'" class="c-center"><p>找不到這位使用者。</p></div>
    <div v-else-if="state === 'error'" class="c-center"><p class="c-err" role="alert">讀取失敗，請稍後再試</p><button class="c-btn" @click="load">重試</button></div>
    <template v-else>
      <section class="c-card">
        <h2 class="c-h2">{{ p.user.display_name }}</h2>
        <p class="c-mute">@{{ p.user.handle }}</p>
        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
        <NuxtLink v-if="p.relation === 'self'" to="/settings" class="c-btn">編輯個人設定</NuxtLink>
        <span v-else-if="p.relation === 'friend'" class="c-badge active">好友</span>
        <span v-else-if="p.relation === 'outgoing'" class="c-badge">已送出邀請</span>
        <button v-else-if="p.relation === 'incoming'" class="c-btn primary" :disabled="busy" @click="accept">接受好友邀請</button>
        <button v-else-if="p.relation === 'none'" class="c-btn primary" :disabled="busy" @click="add">送出好友邀請</button>
        <NuxtLink v-else :to="{ path: '/login', query: { redirect: route.fullPath } }" class="c-btn primary">登入以加好友</NuxtLink>
      </section>

      <section>
        <h2 class="c-h2">心願清單</h2>
        <p v-if="!p.wishlists.length" class="c-mute">目前沒有可看的清單。</p>
        <article v-for="w in p.wishlists" :key="w.slug" class="c-card">
          <div class="c-row">
            <img v-if="w.cover_image_url" class="c-thumb" :src="w.cover_image_url" alt="">
            <div class="c-grow">
              <NuxtLink :to="`/s/${w.slug}`"><strong>{{ w.title }}</strong></NuxtLink>
              <div class="c-row wrap">
                <span class="c-badge">{{ vis[w.visibility] ?? w.visibility }}</span>
                <span v-if="w.status === 'closed'" class="c-badge">已結束</span>
                <span class="c-mute">{{ w.type === 'registry' ? '禮物登記' : '個人心願' }}</span>
              </div>
              <div class="c-mute">完成度 {{ w.completion_pct ?? 0 }}%（{{ w.item_count }} 件）</div>
            </div>
          </div>
        </article>
      </section>

      <section>
        <h2 class="c-h2">捐助紀錄</h2>
        <p v-if="!p.donations.length" class="c-mute">目前沒有可看的捐助紀錄。</p>
        <article v-for="(d, i) in p.donations" :key="i" class="c-card">
          <strong>{{ d.item_title }} × {{ d.qty }}</strong>
          <span class="c-badge">{{ status[d.status] ?? d.status }}</span>
          <div class="c-mute">
            送給 {{ d.wishlist.owner.display_name }}・
            <NuxtLink v-if="d.wishlist.slug" :to="`/s/${d.wishlist.slug}`">{{ d.wishlist.title }}</NuxtLink><template v-else>{{ d.wishlist.title }}</template>
            ・{{ new Date(d.created_at).toLocaleDateString('zh-TW') }}
          </div>
        </article>
      </section>
    </template>
  </main>
</template>
