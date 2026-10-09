<script setup lang="ts">
definePageMeta({ middleware: 'staff' })
useHead({ title: '營運後台' })
const { api } = useApi()
const tab = ref<'reports' | 'lists' | 'users' | 'flags'>('reports')
const err = ref(''), msg = ref('')
const mask = (e: string) => e.replace(/^(.).*(@.*)$/, '$1***$2')

// 檢舉佇列
const rStatus = ref('open')
const reports = ref<any[]>([]), rCursor = ref<string | null>(null), rLoading = ref(false)
const openCount = ref<number | null>(null)
const REASON: Record<string, string> = { scam: '疑似詐騙', inappropriate: '不當內容', copyright: '侵害著作權', personal_info: '洩漏個人資料', other: '其他' }
async function loadReports(more = false) {
  rLoading.value = true; err.value = ''
  try {
    const r = await api('/admin/reports', { query: { status: rStatus.value, cursor: more ? rCursor.value : undefined } })
    reports.value = more ? [...reports.value, ...r.data] : r.data; rCursor.value = r.next_cursor
    if (rStatus.value === 'open' && !r.next_cursor) openCount.value = reports.value.length
  } catch (e) { err.value = errMsg(e) } finally { rLoading.value = false }
}
async function handle(r: any, status: 'actioned' | 'dismissed') {
  err.value = ''
  try { await api(`/admin/reports/${r.id}`, { method: 'PATCH', body: { status } }); await loadReports() }
  catch (e) { err.value = errMsg(e) }
}

// 下架 / 恢復
const mod = ref<{ id: string; title: string; hidden: boolean } | null>(null)
const reason = ref(''), modBusy = ref(false), modErr = ref('')
function openMod(w: any) { mod.value = { id: w.id, title: w.title, hidden: w.moderation_status === 'hidden' }; reason.value = ''; modErr.value = '' }
async function doMod() {
  modBusy.value = true; modErr.value = ''
  try {
    const hide = !mod.value!.hidden
    await api(`/admin/wishlists/${mod.value!.id}/moderation`, { method: 'PATCH', body: hide ? { moderation_status: 'hidden', reason: reason.value.trim() } : { moderation_status: 'ok' } })
    mod.value = null; msg.value = hide ? '已下架' : '已恢復上架'
    await Promise.all([loadReports(), tab.value === 'lists' ? searchLists() : 0])
  } catch (e) { modErr.value = errMsg(e) } finally { modBusy.value = false }
}

// 清單搜尋
const lq = ref(''), lists = ref<any[]>([]), lDone = ref(false)
async function searchLists() {
  err.value = ''
  try { lists.value = (await api('/admin/wishlists', { query: { q: lq.value || undefined } })).data; lDone.value = true }
  catch (e) { err.value = errMsg(e) }
}
// 使用者
const uq = ref(''), users = ref<any[]>([]), shown = ref<Record<string, boolean>>({}), uDone = ref(false)
async function searchUsers() {
  err.value = ''
  try { users.value = (await api('/admin/users', { query: { q: uq.value || undefined } })).data; uDone.value = true; shown.value = {} }
  catch (e) { err.value = errMsg(e) }
}
// 系統旗標
const readOnly = ref<boolean | null>(null), flagAsk = ref(false), flagBusy = ref(false)
async function setFlag() {
  flagBusy.value = true; err.value = ''
  try { const r = await api('/admin/system-flags/read_only', { method: 'PUT', body: { value: !readOnly.value } }); readOnly.value = r.value; flagAsk.value = false }
  catch (e) { err.value = errMsg(e) } finally { flagBusy.value = false }
}
async function loadFlags() {
  try { readOnly.value = !!(await api('/admin/system-flags')).data.find((f: any) => f.key === 'read_only')?.value } catch {}
}
watch(rStatus, () => loadReports())
watch(tab, t => { if (t === 'flags') loadFlags() })
onMounted(() => loadReports())
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader title="營運後台" back="/dashboard"><span class="c-badge">STAFF</span></CreatorHeader>
    <div class="c-row wrap c-mb">
      <div class="c-card c-grow c-center c-m0"><div class="c-big">{{ openCount ?? '–' }}</div><div class="c-mute">待處理檢舉</div></div>
    </div>
    <nav class="c-tabs" aria-label="後台功能">
      <button v-for="t in [['reports', '檢舉佇列'], ['lists', '清單搜尋'], ['users', '使用者'], ['flags', '系統旗標']]" :key="t[0]" class="c-btn" :class="{ on: tab === t[0] }" @click="tab = t[0] as any">{{ t[1] }}</button>
    </nav>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>

    <section v-if="tab === 'reports'">
      <label class="c-field"><span>狀態</span><select v-model="rStatus"><option value="open">待處理</option><option value="actioned">已處理</option><option value="dismissed">已駁回</option></select></label>
      <p v-if="rLoading" class="c-mute">載入中…</p>
      <p v-else-if="!reports.length" class="c-center c-mute">目前沒有{{ rStatus === 'open' ? '待處理的' : '' }}檢舉</p>
      <article v-for="r in reports" :key="r.id" class="c-card">
        <div class="c-row"><strong class="c-grow">{{ REASON[r.reason] || r.reason }}</strong><CreatorStatusBadge :v="r.status" /></div>
        <div>清單：<a :href="`/s/${r.wishlist.slug}`" target="_blank" rel="noopener">{{ r.wishlist.title }}</a>（{{ r.wishlist.slug }}）</div>
        <div v-if="r.detail" class="c-mute">{{ r.detail }}</div>
        <div class="c-mute">{{ new Date(r.created_at).toLocaleString('zh-TW') }}・檢舉者：{{ r.reporter }}{{ r.item_id ? '・針對品項' : '' }}</div>
        <div v-if="r.status === 'open'" class="c-row wrap c-mt">
          <button class="c-btn danger c-grow" @click="openMod({ id: r.wishlist.id, title: r.wishlist.title, moderation_status: 'ok' })">下架</button>
          <button class="c-btn c-grow" @click="handle(r, 'actioned')">已處理</button>
          <button class="c-btn c-grow" @click="handle(r, 'dismissed')">駁回</button>
        </div>
      </article>
      <button v-if="rCursor" class="c-btn block" @click="loadReports(true)">載入更多</button>
    </section>

    <section v-if="tab === 'lists'">
      <form class="c-row" @submit.prevent="searchLists"><input v-model="lq" class="c-grow c-input" placeholder="slug 或建立者 Email 前綴" aria-label="搜尋清單"><button class="c-btn primary">搜尋</button></form>
      <p v-if="lDone && !lists.length" class="c-center c-mute">沒有符合的清單</p>
      <article v-for="w in lists" :key="w.id" class="c-card c-mt12">
        <div class="c-row"><strong class="c-grow">{{ w.title }}</strong><CreatorStatusBadge :v="w.moderation_status === 'hidden' ? 'hidden' : w.status" /></div>
        <div class="c-mute">{{ w.slug }}・{{ mask(w.owner.email || '') }}・被檢舉 {{ w.open_report_count }} 次</div>
        <div class="c-row c-mt">
          <a :href="`/s/${w.slug}`" target="_blank" rel="noopener" class="c-btn">公開頁</a>
          <button class="c-btn c-grow" :class="{ danger: w.moderation_status !== 'hidden' }" @click="openMod(w)">{{ w.moderation_status === 'hidden' ? '恢復上架' : '下架' }}</button>
        </div>
      </article>
    </section>

    <section v-if="tab === 'users'">
      <form class="c-row" @submit.prevent="searchUsers"><input v-model="uq" class="c-grow c-input" placeholder="Email 前綴或 id" aria-label="搜尋使用者"><button class="c-btn primary">搜尋</button></form>
      <p v-if="uDone && !users.length" class="c-center c-mute">沒有符合的使用者</p>
      <article v-for="u in users" :key="u.id" class="c-card c-mt12">
        <strong>{{ u.display_name }}</strong> <span v-if="u.is_staff" class="c-badge">STAFF</span> <span v-if="u.blocked" class="c-badge hidden">已封鎖</span> <span v-if="u.deleted" class="c-badge">已刪除</span>
        <div class="c-mute">{{ shown[u.id] ? u.email : mask(u.email || '') }} <button v-if="u.email" class="c-btn small" @click="shown[u.id] = !shown[u.id]">{{ shown[u.id] ? '隱藏' : '顯示' }}</button></div>
        <div class="c-mute">清單 {{ u.wishlist_count }}・{{ new Date(u.created_at).toLocaleDateString('zh-TW') }}</div>
      </article>
    </section>

    <section v-if="tab === 'flags'" class="c-card">
      <label class="c-switch"><input type="checkbox" :checked="!!readOnly" @click.prevent="flagAsk = true"><span>唯讀模式（read_only）<br><small class="c-mute">開啟後所有非安全方法回 503，全站無法寫入。目前狀態{{ readOnly === null ? '未知（切換後顯示）' : readOnly ? '開啟' : '關閉' }}。</small></span></label>
      <p class="c-mute">所有營運動作都會寫入稽核紀錄。</p>
    </section>

    <CreatorConfirm :open="!!mod" :title="mod?.hidden ? '恢復上架' : '下架清單'" :text="`清單：${mod?.title}`" :ok="mod?.hidden ? '恢復上架' : '下架此清單'" :danger="!mod?.hidden" :busy="modBusy" :disabled="!mod?.hidden && !reason.trim()" :error="modErr" @close="mod = null" @ok="doMod">
      <label v-if="mod && !mod.hidden" class="c-field"><span>下架原因（必填，會顯示給建立者）</span><input v-model="reason" maxlength="200"></label>
    </CreatorConfirm>
    <CreatorConfirm :open="flagAsk" title="切換唯讀模式" :text="readOnly ? '確定關閉唯讀模式？' : '唯讀模式會影響全站寫入，確定開啟？'" :busy="flagBusy" danger @close="flagAsk = false" @ok="setFlag" />
  </main>
</template>
