<script setup lang="ts">
// bottom sheet：原生 <dialog>（焦點鎖定、Esc 關閉、關閉後焦點回觸發元素由瀏覽器處理）
const props = defineProps<{ open: boolean; title: string }>()
const emit = defineEmits<{ close: [] }>()
const el = ref<HTMLDialogElement>()
watch(() => props.open, (v) => {
  if (!el.value) return
  if (v && !el.value.open) el.value.showModal()
  if (!v && el.value.open) el.value.close()
})
</script>
<template>
  <dialog ref="el" class="c-sheet" :aria-label="title" @close="emit('close')">
    <template v-if="open">
      <h2>{{ title }}</h2>
      <slot />
    </template>
  </dialog>
</template>
