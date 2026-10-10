<script setup lang="ts">
defineProps<{ title: string; back?: string }>()
const { user } = useAuth()
</script>
<template>
  <header class="c-head">
    <NuxtLink v-if="back" :to="back" class="c-btn plain" aria-label="返回">‹</NuxtLink>
    <h1>{{ title }}</h1>
    <NuxtLink v-if="user?.handle" :to="`/u/${user.handle}`" class="c-btn">個人頁</NuxtLink>
    <NuxtLink v-if="user" to="/friends" class="c-btn">好友</NuxtLink>
    <slot />
  </header>
</template>
