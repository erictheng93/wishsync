<script setup lang="ts">
import '~/assets/tokens.css'
import '~/assets/guest.css'
// 全站錯誤頁（取代 Nuxt 預設英文頁）。SSR 狀態碼由 Nuxt 依 error.statusCode 回傳，這裡只管畫面。
// 403 刻意與 404 同樣呈現：/admin 對非營運人員要「看起來像不存在」。5xx 不顯示內部 message。
const props = defineProps<{ error: { statusCode?: number, status?: number } }>()
const code = computed(() => props.error?.statusCode ?? props.error?.status ?? 500)
const missing = computed(() => code.value === 404 || code.value === 403)
useSeoMeta({ title: () => missing.value ? '找不到頁面' : '發生錯誤', robots: 'noindex' })
</script>

<template>
  <GuestTop />
  <main class="g-wrap g-center" style="padding-inline:16px">
    <template v-if="missing">
      <h1>找不到頁面</h1>
      <p class="g-mute">這個網址不存在，或已經移除。</p>
    </template>
    <template v-else>
      <h1>系統發生問題</h1>
      <p class="g-mute">請稍後再試；若持續發生，請回報給我們。</p>
    </template>
    <button class="g-btn" @click="clearError({ redirect: '/' })">回首頁</button>
  </main>
</template>
