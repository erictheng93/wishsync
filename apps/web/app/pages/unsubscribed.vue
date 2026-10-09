<script setup lang="ts">
import '~/assets/guest.css'
useSeoMeta({ title: '通知設定', robots: 'noindex' })
const ok = useRoute().query.ok === '1'
</script>
<template>
  <GuestTop />
  <main class="g-wrap g-center" style="padding-inline:16px">
    <h1>{{ ok ? '已取消通知' : '連結已失效' }}</h1>
    <p class="g-mute">{{ ok ? '之後不會再寄「有人認領」的通知信。想重新開啟，登入後到「帳號設定」即可。' : '這個退訂連結無效或已過期，請登入後到「帳號設定」關閉通知。' }}</p>
    <NuxtLink to="/settings">前往帳號設定</NuxtLink>
  </main>
</template>
