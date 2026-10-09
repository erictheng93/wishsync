<script setup lang="ts">
// 6 格驗證碼輸入；v-model 為字串，填滿 6 碼 emit('complete')
const model = defineModel<string>({ default: '' })
const emit = defineEmits<{ complete: [code: string] }>()
const cells = ref<HTMLInputElement[]>([])
const digit = (i: number) => model.value[i] || ''
function set(i: number, c: string) {
  const a = Array.from({ length: 6 }, (_, k) => digit(k)); a[i] = c; return (model.value = a.join(''))
}
function onInput(i: number, ev: Event) {
  const v = (ev.target as HTMLInputElement).value.replace(/\D/g, '')
  let next = model.value
  if (v.length > 1) { // 貼上整串
    next = model.value = v.slice(0, 6)
    cells.value[Math.min(v.length, 5)]?.focus()
  } else {
    next = set(i, v)
    if (v && i < 5) cells.value[i + 1]?.focus()
  }
  if (next.length === 6) emit('complete', next)
}
function onKey(i: number, ev: KeyboardEvent) {
  if (ev.key === 'Backspace' && !digit(i) && i > 0) cells.value[i - 1]?.focus()
}
defineExpose({ focus: () => cells.value[0]?.focus() })
</script>
<template>
  <div class="c-otp" role="group" aria-label="6 位數驗證碼">
    <input v-for="i in 6" :key="i" :ref="(el) => (cells[i - 1] = el as HTMLInputElement)" :value="digit(i - 1)" inputmode="numeric" autocomplete="one-time-code" maxlength="6" :aria-label="`第 ${i} 碼`" @input="onInput(i - 1, $event)" @keydown="onKey(i - 1, $event)">
  </div>
</template>
