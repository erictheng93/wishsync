<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
useHead({ title: '建立清單' })
const { api } = useApi()
const f = reactive({ type: 'registry', title: '', description: '', event_date: '', surprise_mode: false, show_claimer_names: false })
const cover = ref<string | null>(null)
const upBusy = ref(false), busy = ref(false), err = ref(''), dateErr = ref('')
const today = new Date(Date.now() + 8 * 3600e3).toISOString().slice(0, 10)
async function submit() {
  err.value = dateErr.value = ''
  if (f.surprise_mode && !f.event_date) { dateErr.value = '開啟驚喜模式需設定活動日'; return }
  if (f.surprise_mode && f.event_date <= today) { dateErr.value = '活動日需為今日之後'; return }
  busy.value = true
  try {
    const body: any = { type: f.type, title: f.title.trim(), description: f.description || null, event_date: f.event_date || null, visibility: 'link', surprise_mode: f.surprise_mode, show_claimer_names: f.show_claimer_names }
    if (cover.value) body.cover_image_key = cover.value
    const w = await api('/wishlists', { method: 'POST', body })
    await navigateTo(`/lists/${w.id}/edit`)
  } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader title="建立清單" back="/dashboard" />
    <form @submit.prevent="submit">
      <div class="c-field"><span>清單類型</span>
        <div class="c-seg">
          <label><input v-model="f.type" type="radio" value="personal"><span>個人心願</span></label>
          <label><input v-model="f.type" type="radio" value="registry"><span>禮物登記</span></label>
        </div>
      </div>
      <label class="c-field"><span>清單名稱（{{ f.title.length }} / 100）</span><input v-model="f.title" maxlength="100" required placeholder="例如：小愛的待產清單"></label>
      <label class="c-field"><span>說明（選填）</span><textarea v-model="f.description" rows="2" maxlength="300" /></label>
      <CreatorImageUploader v-model="cover" purpose="cover" @busy="upBusy = $event" />
      <label class="c-field"><span>活動日（選填）</span><input v-model="f.event_date" type="date" :min="today"></label>
      <p v-if="dateErr" class="c-err" role="alert">{{ dateErr }}</p>
      <label class="c-switch"><input v-model="f.surprise_mode" type="checkbox"><span>驚喜模式<br><small class="c-mute">活動日前，你只看得到完成度，看不到誰送了什麼。需設定活動日。</small></span></label>
      <label class="c-switch"><input v-model="f.show_claimer_names" type="checkbox"><span>顯示認領者暱稱給其他訪客<br><small class="c-mute">預設關閉</small></span></label>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block" :disabled="busy || upBusy || !f.title.trim()">{{ busy ? '建立中…' : '建立並新增品項' }}</button>
    </form>
  </main>
</template>
