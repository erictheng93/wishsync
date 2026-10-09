<script setup lang="ts">
// 選圖 -> 檢查型別/大小 -> 最長邊 1280 壓縮 -> presign -> PUT。modelValue = object_key
const props = defineProps<{ purpose: 'cover' | 'item'; modelValue?: string | null; currentUrl?: string | null }>()
const emit = defineEmits<{ 'update:modelValue': [string | null]; busy: [boolean] }>()
const { api } = useApi()
const MAX = 5 * 1024 * 1024
const preview = ref<string | null>(null)
const state = ref<'idle' | 'uploading' | 'done' | 'fail'>('idle')
const msg = ref('')
let last: File | null = null

async function shrink(f: File): Promise<Blob> {
  try {
    const bmp = await createImageBitmap(f, { imageOrientation: 'from-image' })
    const k = Math.min(1, 1280 / Math.max(bmp.width, bmp.height))
    const c = document.createElement('canvas')
    c.width = Math.round(bmp.width * k); c.height = Math.round(bmp.height * k)
    c.getContext('2d')!.drawImage(bmp, 0, 0, c.width, c.height)
    const b: Blob | null = await new Promise(r => c.toBlob(r, 'image/webp', 0.85))
    if (b && b.type === 'image/webp') return b
  } catch {}
  return f
}
async function upload(f: File) {
  last = f; msg.value = ''
  if (!['image/jpeg', 'image/png', 'image/webp'].includes(f.type)) { state.value = 'fail'; msg.value = '圖片限 JPG、PNG、WebP'; return }
  if (f.size > MAX * 4) { state.value = 'fail'; msg.value = '圖片需在 5 MB 以內'; return }
  state.value = 'uploading'; emit('busy', true)
  try {
    const blob = await shrink(f)
    if (blob.size > MAX) throw new Error('圖片需在 5 MB 以內')
    const p = await api('/uploads/presign', { method: 'POST', body: { purpose: props.purpose, content_type: blob.type, content_length: blob.size } })
    const headers: Record<string, string> = {}
    for (const [k, v] of Object.entries(p.headers || {})) if (k.toLowerCase() !== 'content-length') headers[k] = v as string
    const r = await fetch(p.upload_url, { method: p.method || 'PUT', headers, body: blob })
    if (!r.ok) throw new Error('上傳失敗')
    await api('/uploads/confirm', { method: 'POST', body: { object_key: p.object_key } }) // 後端去 EXIF 並寫入公開 key
    if (preview.value) URL.revokeObjectURL(preview.value)
    preview.value = URL.createObjectURL(blob)
    emit('update:modelValue', p.object_key)
    state.value = 'done'
  } catch (e: any) {
    state.value = 'fail'; msg.value = e instanceof CreatorApiError ? errMsg(e) : e.message || '上傳失敗'
  } finally { emit('busy', false) }
}
function pick(ev: Event) {
  const f = (ev.target as HTMLInputElement).files?.[0]
  if (f) upload(f)
}
</script>
<template>
  <div class="c-field">
    <span>{{ purpose === 'cover' ? '封面圖片（選填）' : '圖片（選填）' }}</span>
    <div class="c-row">
      <img v-if="preview || currentUrl" class="c-thumb" :src="preview || currentUrl!" alt="">
      <div v-else class="c-thumb">無圖</div>
      <div class="c-grow">
        <label class="c-btn c-file" :aria-disabled="state === 'uploading'">
          選擇圖片
          <input type="file" accept="image/jpeg,image/png,image/webp" :disabled="state === 'uploading'" @change="pick">
        </label>
        <div v-if="state === 'uploading'" class="c-mute" role="status">上傳中…</div>
        <div v-else-if="state === 'done'" class="c-ok">已上傳，圖片處理完成後才會公開顯示</div>
        <div v-else-if="state === 'fail'" class="c-err" role="alert">{{ msg }} <button v-if="last" type="button" class="c-btn" @click="upload(last!)">重新上傳</button></div>
      </div>
    </div>
    <p class="c-mute">限 JPG、PNG、WebP，5 MB 以內。上傳後會移除位置等隱藏資訊（EXIF），請勿上傳含個人資料的照片。</p>
  </div>
</template>
