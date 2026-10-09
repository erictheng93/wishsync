<script setup lang="ts">
defineProps<{ open: boolean; title: string; text?: string; ok?: string; danger?: boolean; busy?: boolean; disabled?: boolean; error?: string }>()
defineEmits<{ close: []; ok: [] }>()
</script>
<template>
  <CreatorSheet :open="open" :title="title" @close="$emit('close')">
    <p v-if="text">{{ text }}</p>
    <p v-if="error" class="c-err" role="alert">{{ error }}</p>
    <slot />
    <div class="c-row">
      <button type="button" class="c-btn c-grow" @click="$emit('close')">取消</button>
      <button type="button" class="c-btn c-grow" :class="danger ? 'danger' : 'primary'" :disabled="busy || disabled" @click="$emit('ok')">{{ busy ? '處理中…' : ok || '確認' }}</button>
    </div>
  </CreatorSheet>
</template>
