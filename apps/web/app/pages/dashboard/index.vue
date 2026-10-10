<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
useHead({ title: '我的清單' })
const { api } = useApi()
const { user } = useAuth()
const list = ref<any[]>([])
const cursor = ref<string | null>(null)
const loading = ref(true), err = ref('')
const del = ref<any>(null), delBusy = ref(false), delErr = ref('')
async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/wishlists', { query: { limit: 20, cursor: more ? cursor.value : undefined } })
    list.value = more ? [...list.value, ...r.data] : r.data
    cursor.value = r.next_cursor
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
async function archive() {
  delBusy.value = true; delErr.value = ''
  try { await api(`/wishlists/${del.value.id}`, { method: 'DELETE' }); del.value = null; await load() }
  catch (e) { delErr.value = errMsg(e) } finally { delBusy.value = false }
}
onMounted(() => load())
const visible = computed(() => list.value.filter(w => w.status !== 'archived'))
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="我的清單">
      <NuxtLink v-if="user?.is_staff" to="/admin" class="c-btn">後台</NuxtLink>
      <NuxtLink to="/me/wallet" class="c-btn">點數</NuxtLink>
      <NuxtLink to="/settings" class="c-btn" aria-label="設定">設定</NuxtLink>
    </CreatorHeader>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">重試</button></p>
    <p v-else-if="loading && !list.length" class="c-mute" role="status">載入中…</p>
    <div v-else-if="!visible.length" class="c-center">
      <p class="c-mute">還沒有清單，建立第一份吧</p>
      <NuxtLink to="/lists/new" class="c-btn primary">＋ 建立清單</NuxtLink>
    </div>
    <article v-for="w in visible" :key="w.id" class="c-card" :class="{ bad: w.moderation_status === 'hidden' }">
      <div class="c-row">
        <img v-if="w.cover_image_url" class="c-thumb" :src="w.cover_image_url" alt="">
        <div class="c-grow">
          <NuxtLink :to="`/lists/${w.id}/edit`"><strong>{{ w.title }}</strong></NuxtLink>
          <div class="c-row wrap">
            <CreatorStatusBadge :v="w.moderation_status === 'hidden' ? 'hidden' : w.status" />
            <span class="c-mute">{{ w.type === 'registry' ? '禮物登記' : '個人心願' }}</span>
          </div>
          <div class="c-mute">完成度 {{ w.completion?.completion_pct ?? 0 }}%（{{ w.completion?.fulfilled_count ?? 0 }} / {{ w.completion?.item_count ?? 0 }}）</div>
        </div>
      </div>
      <div v-if="w.moderation_status === 'hidden'" class="c-err" role="alert">
        這份清單已被下架{{ w.moderation_reason ? `。原因：${w.moderation_reason}` : '' }}。公開頁目前無法瀏覽，分享已停用。若有誤判請聯絡客服申訴。
      </div>
      <div class="c-row c-mt">
        <NuxtLink :to="`/lists/${w.id}/edit`" class="c-btn c-grow">編輯</NuxtLink>
        <NuxtLink :to="`/lists/${w.id}/progress`" class="c-btn c-grow">進度</NuxtLink>
        <button class="c-btn danger" @click="del = w; delErr = ''">封存</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">載入更多</button>
    <NuxtLink v-if="visible.length" to="/lists/new" class="c-btn primary c-fab">＋ 新建</NuxtLink>
    <CreatorConfirm :open="!!del" title="封存清單" :text="`封存「${del?.title}」後，公開頁會立即失效，朋友無法再開啟（既有認領仍保留）。`" ok="確認封存" danger :busy="delBusy" :error="delErr" @close="del = null" @ok="archive" />
  </main>
</template>
