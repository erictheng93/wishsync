<script setup lang="ts">
defineProps<{ label: string; autocomplete: 'current-password' | 'new-password'; hint?: string }>()
const model = defineModel<string>({ default: '' })
const show = ref(false)
const id = useId()
</script>
<template>
  <div class="c-field">
    <label :for="id" class="c-lbl">{{ label }}</label>
    <div class="c-pw">
      <input :id="id" v-model="model" :type="show ? 'text' : 'password'" :autocomplete="autocomplete" :aria-describedby="hint ? id + '-h' : undefined" required>
      <button type="button" class="c-btn plain small" :aria-pressed="show" @click="show = !show">{{ show ? '隱藏' : '顯示' }}</button>
    </div>
    <small v-if="hint" :id="id + '-h'" class="c-mute">{{ hint }}</small>
  </div>
</template>
