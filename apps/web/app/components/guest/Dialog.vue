<script setup lang="ts">
import '~/assets/guest.css'
// 訪客端 bottom sheet 的 modal 外殼：原生 <dialog>.showModal()（焦點鎖定、Esc、inert 背景由瀏覽器處理）。
// 由父層 v-if 掛載即開啟；dirty 時 Esc / 點背景要二次確認，避免丟掉已輸入內容。
const props = defineProps<{ labelledby: string, dirty?: boolean }>()
const emit = defineEmits<{ close: [] }>()
const el = ref<HTMLDialogElement>()
let opener: HTMLElement | null = null
const leave = () => !props.dirty || confirm('放棄已輸入的內容？')
onBeforeMount(() => { opener = document.activeElement as HTMLElement | null })
onMounted(() => {
  el.value!.showModal()
  // 初始焦點：第一個可輸入欄位，沒有就第一個連結 / 按鈕
  const q = (sel: string) => el.value!.querySelector<HTMLElement>(sel)
  ;(q('input:not([type=hidden]):not([readonly]), textarea') ?? q('a[href], button:not(:disabled)'))?.focus()
})
// 父層 v-if 移除時 <dialog> 直接消失，瀏覽器不會還焦點；且 modal 開著時背景是 inert，所以先 close() 再自己還
onBeforeUnmount(() => { el.value?.close(); if (opener?.isConnected) opener.focus() })
</script>

<template>
  <dialog ref="el" class="g-dialog" :aria-labelledby="labelledby"
    @cancel.prevent="leave() && emit('close')" @close="emit('close')" @click.self="leave() && emit('close')">
    <slot />
  </dialog>
</template>
