<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
useHead({ title: '編輯清單' })
const route = useRoute()
const id = route.params.id as string
const { api } = useApi()
const w = ref<any>(null)
const items = ref<any[]>([])
const loading = ref(true), err = ref(''), msg = ref(''), stale = ref(false)
const sheet = ref(false), editing = ref<any>(null)
const del = ref<any>(null), delBusy = ref(false), delErr = ref('')
const pubBusy = ref(false), pubErrs = ref<string[]>([])
const external = ref(false), copied = ref(false), showShare = ref(false)
const meta = reactive<any>({ title: '', description: '', event_date: '', show_claimer_names: false })
let poll: any

async function load(quiet = false) {
  if (!quiet) loading.value = true
  try {
    const r = await api(`/wishlists/${id}`)
    w.value = r.wishlist; items.value = r.items
    Object.assign(meta, { title: r.wishlist.title, description: r.wishlist.description ?? '', event_date: r.wishlist.event_date ?? '', show_claimer_names: r.wishlist.show_claimer_names })
    err.value = ''; stale.value = false
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
  clearTimeout(poll)
  if (items.value.some(i => i.image_status === 'pending')) poll = setTimeout(() => load(true), 4000) // 圖片處理中輪詢
}
onMounted(() => load())
onBeforeUnmount(() => clearTimeout(poll))

const hidden = computed(() => w.value?.moderation_status === 'hidden')
const shareUrl = computed(() => {
  if (!w.value) return ''
  const u = w.value.share_url || `${location.origin}/s/${w.value.slug}`
  return external.value ? u + '?openExternalBrowser=1' : u
})
function openItem(i: any = null) { editing.value = i; sheet.value = true }
async function saved() { sheet.value = false; await load(true) }
async function patchList(body: any, ok = '已儲存') {
  msg.value = ''; err.value = ''; stale.value = false
  try { w.value = await api(`/wishlists/${id}`, { method: 'PATCH', body: { ...body, expected_updated_at: w.value?.updated_at } }); msg.value = ok; return true }
  catch (e: any) {
    if (e.code === 'STALE_VERSION') { stale.value = true; err.value = '這份清單已在其他地方被修改，請重新載入'; return false }
    err.value = errMsg(e)
    if (e.code === 'WISHLIST_NOT_PUBLISHABLE') pubErrs.value = (e.errors || []).map((x: any) => x.detail)
    return false
  }
}
async function saveMeta() {
  await patchList({ title: meta.title.trim(), description: meta.description || null, event_date: meta.event_date || null, show_claimer_names: meta.show_claimer_names })
}
async function publish() {
  pubBusy.value = true; pubErrs.value = []
  if (await patchList({ status: 'active' }, '已發佈')) showShare.value = true
  pubBusy.value = false
}
async function remove(force = false) {
  delBusy.value = true; delErr.value = ''
  try { await api(`/items/${del.value.id}`, { method: 'DELETE', query: force ? { force: true } : {} }); del.value = null; await load(true) }
  catch (e: any) {
    if (e.code === 'ITEM_HAS_CLAIMS' && !force) delErr.value = '此品項已有認領。若仍要刪除，認領將一併取消，請再按一次確認。'
    else delErr.value = errMsg(e)
    if (e.code === 'ITEM_HAS_CLAIMS') del.value.force = true
  } finally { delBusy.value = false }
}
async function move(i: number, d: number) {
  const a = [...items.value]; const j = i + d
  if (j < 0 || j >= a.length) return
  ;[a[i], a[j]] = [a[j], a[i]]
  items.value = a
  try { await api(`/wishlists/${id}/items/reorder`, { method: 'POST', body: { item_ids: a.map(x => x.id) } }) }
  catch (e) { err.value = errMsg(e); await load(true) }
}
async function copy() {
  try { await navigator.clipboard.writeText(shareUrl.value); copied.value = true; setTimeout(() => (copied.value = false), 2000) }
  catch { (document.getElementById('c-share-input') as HTMLInputElement)?.select() }
}
const lineHref = computed(() => `https://line.me/R/share?text=${encodeURIComponent(`${w.value?.title ?? ''} ${shareUrl.value}`)}`)
const closed = computed(() => w.value && ['closed', 'archived'].includes(w.value.status))
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="w?.title || '編輯清單'" back="/dashboard">
      <CreatorStatusBadge v-if="w" :v="w.status" />
      <NuxtLink :to="`/lists/${id}/progress`" class="c-btn">進度</NuxtLink>
    </CreatorHeader>
    <p v-if="loading" class="c-mute" role="status">載入中…</p>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ stale ? '重新載入' : '重試' }}</button></p>
    <template v-if="w">
      <div v-if="hidden" class="c-card bad c-err" role="alert">這份清單已被下架{{ w.moderation_reason ? `。原因：${w.moderation_reason}` : '' }}。分享已停用，若有誤判請聯絡客服申訴。</div>
      <p v-if="closed" class="c-mute">清單已{{ w.status === 'closed' ? '結束' : '封存' }}，品項無法修改。</p>
      <ul v-if="pubErrs.length" class="c-err"><li v-for="x in pubErrs" :key="x">{{ x }}</li></ul>

      <p v-if="!items.length && !loading" class="c-center c-mute">還沒有品項，先新增第一個吧</p>
      <article v-for="(i, k) in items" :key="i.id" class="c-card">
        <div class="c-row">
          <img v-if="i.image_url" class="c-thumb" :src="i.image_url" alt="">
          <div v-else class="c-thumb">{{ i.image_status === 'pending' ? '處理中' : '無圖' }}</div>
          <div class="c-grow">
            <strong>{{ i.title }}</strong>
            <div class="c-mute">需要 {{ i.qty_needed }}・數量認領・優先度{{ { high: '高', medium: '中', low: '低' }[i.priority as string] || '' }}</div>
            <CreatorStatusBadge v-if="i.image_status === 'pending' || i.image_status === 'rejected'" :v="i.image_status" />
            <div v-if="i.image_status === 'rejected'" class="c-err">圖片未通過，請換一張</div>
          </div>
        </div>
        <div v-if="!closed" class="c-row wrap c-mt">
          <button class="c-btn" :disabled="k === 0" aria-label="上移" @click="move(k, -1)">↑</button>
          <button class="c-btn" :disabled="k === items.length - 1" aria-label="下移" @click="move(k, 1)">↓</button>
          <button class="c-btn c-grow" @click="openItem(i)">編輯</button>
          <button class="c-btn danger" @click="del = { ...i }; delErr = ''">刪除</button>
        </div>
      </article>
      <button v-if="!closed" class="c-btn block c-mb16" @click="openItem()">＋ 新增品項</button>

      <details class="c-card">
        <summary>清單設定</summary>
        <form class="c-mt12" @submit.prevent="saveMeta">
          <label class="c-field"><span>清單名稱</span><input v-model="meta.title" maxlength="40" required></label>
          <label class="c-field"><span>說明</span><textarea v-model="meta.description" rows="2" /></label>
          <label class="c-field"><span>活動日</span><input v-model="meta.event_date" type="date"></label>
          <label class="c-switch"><input v-model="meta.show_claimer_names" type="checkbox"><span>顯示認領者暱稱給其他訪客</span></label>
          <p class="c-mute">驚喜模式：{{ w.surprise_mode ? (w.surprise_locked ? '開啟中（鎖定至活動日，無法關閉）' : '開啟中（已解鎖）') : '未開啟' }}</p>
          <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
          <button class="c-btn block">儲存設定</button>
        </form>
        <div class="c-row c-mt">
          <button v-if="w.status === 'active'" class="c-btn c-grow" @click="patchList({ status: 'closed' }, '清單已結束')">結束清單</button>
          <button v-if="w.status === 'closed'" class="c-btn c-grow" @click="patchList({ status: 'active' }, '已重新開放')">重新開放</button>
        </div>
      </details>

      <div class="c-bottom"><div>
        <button v-if="w.status === 'draft'" class="c-btn primary c-grow" :disabled="pubBusy || !items.length" @click="publish">{{ pubBusy ? '發佈中…' : '發佈並分享' }}</button>
        <button v-else class="c-btn primary c-grow" :disabled="hidden" @click="showShare = true">分享</button>
      </div></div>
    </template>

    <CreatorItemSheet :open="sheet" :wishlist-id="id" :item="editing" @close="sheet = false" @saved="saved" />
    <CreatorConfirm :open="!!del" title="刪除品項" :text="`確定刪除「${del?.title}」？`" :ok="del?.force ? '連同認領一併刪除' : '刪除'" danger :busy="delBusy" :error="delErr" @close="del = null" @ok="remove(!!del?.force)" />
    <CreatorSheet :open="showShare" title="分享你的清單" @close="showShare = false">
      <p v-if="w?.status === 'draft'" class="c-err">尚未發佈，朋友打開會看到不存在。</p>
      <input id="c-share-input" class="c-input c-mb" :value="shareUrl" readonly @focus="($event.target as HTMLInputElement).select()">
      <label class="c-switch"><input v-model="external" type="checkbox"><span>連結加上 openExternalBrowser=1（在 LINE 內改用預設瀏覽器開啟）</span></label>
      <div class="c-row wrap">
        <button class="c-btn c-grow" @click="copy">{{ copied ? '已複製 ✓' : '複製連結' }}</button>
        <a class="c-btn c-grow" :href="lineHref" target="_blank" rel="noopener">用 LINE 傳</a>
        <a class="c-btn c-grow" :href="`sms:?&body=${encodeURIComponent(shareUrl)}`">簡訊</a>
        <a class="c-btn c-grow" :href="`https://www.facebook.com/sharer/sharer.php?u=${encodeURIComponent(shareUrl)}`" target="_blank" rel="noopener">Facebook</a>
      </div>
      <button class="c-btn block c-mt" @click="showShare = false">關閉</button>
    </CreatorSheet>
  </main>
</template>
