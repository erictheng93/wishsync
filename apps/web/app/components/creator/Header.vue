<script setup lang="ts">
defineProps<{ title: string; back?: string }>()
</script>
<template>
  <header class="c-head">
    <NuxtLink v-if="back" :to="back" class="c-btn plain" aria-label="返回">‹</NuxtLink>
    <h1>{{ title }}</h1>
    <slot />
  </header>
</template>
