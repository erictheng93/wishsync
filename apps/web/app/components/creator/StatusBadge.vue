<script setup lang="ts">
const props = defineProps<{ v: string }>()
const L: Record<string, string> = { draft: '草稿', active: '進行中', closed: '已結束', archived: '已封存', hidden: '已下架', ok: '正常', pending: '圖片處理中', ready: '圖片就緒', rejected: '圖片未通過', reserved: '已認領', purchased: '已購買', delivered: '已送達', cancelled: '已取消', open: '待處理', actioned: '已處理', dismissed: '已駁回' }
</script>
<template><span :class="['c-badge', v]">{{ L[v] || v }}</span></template>
