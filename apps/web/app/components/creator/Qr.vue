<script setup lang="ts">
import { renderSVG } from 'uqr'
const props = withDefaults(defineProps<{ text: string, size?: number }>(), { size: 200 })
const svg = computed(() => props.text ? renderSVG(props.text, { ecc: 'M', border: 2 }) : '')
function download() {
  const url = URL.createObjectURL(new Blob([svg.value], { type: 'image/svg+xml' }))
  const a = Object.assign(document.createElement('a'), { href: url, download: 'wishsync-qr.svg' })
  a.click(); URL.revokeObjectURL(url)
}
</script>
<template>
  <div class="c-qr">
    <!-- eslint-disable-next-line vue/no-v-html -- uqr 自行產生的 SVG，非使用者輸入 -->
    <div role="img" aria-label="分享連結 QR code" :style="{ width: size + 'px', height: size + 'px' }" v-html="svg" />
    <button type="button" class="c-btn" :disabled="!svg" @click="download">下載 QR</button>
  </div>
</template>
<style scoped>
.c-qr{display:flex;flex-direction:column;align-items:center;gap:8px;margin:12px 0}
.c-qr :deep(svg){width:100%;height:100%;display:block;background:#fff;border:1px solid var(--line)}
</style>
