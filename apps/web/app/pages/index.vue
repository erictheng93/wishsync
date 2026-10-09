<script setup lang="ts">
const { user, fetchMe } = useAuth()
onMounted(() => fetchMe())
</script>
<template>
  <main class="c-page c-hero">
    <h1>WishSync 心願同步</h1>
    <p class="c-mute">列出你想要的，讓想送的人不再猜。</p>
    <NuxtLink v-if="user" to="/dashboard" class="c-btn primary block">我的清單</NuxtLink>
    <NuxtLink v-else to="/login" class="c-btn primary block">開始建立</NuxtLink>
    <p class="c-mute"><NuxtLink to="/terms">服務條款</NuxtLink>・<NuxtLink to="/privacy">隱私權政策</NuxtLink></p>
  </main>
</template>
