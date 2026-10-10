<script setup lang="ts">
import '~/assets/guest.css'
const props = defineProps<{ item: any, mine: any | null, masked: boolean, closed: boolean, online: boolean }>()
defineEmits<{ claim: [], fund: [] }>()
const prio: Record<string, string> = { high: '高', medium: '中', low: '低' }
const crowd = computed(() => props.item.funding_mode === 'crowdfund')
const ds = computed(() => deriveDisplayStatus(props.item) ?? '')
const full = computed(() => crowd.value ? ds.value !== 'open' : props.item.is_fully_claimed)
// image_status 非 ready 時 API 不給 image_url，顯示灰色佔位
</script>
<template>
  <li class="g-card g-item" :class="{ full }">
    <img v-if="item.image_url" class="g-thumb" :src="item.image_url" alt="" loading="lazy">
    <div v-else class="g-thumb">圖</div>
    <div class="g-body">
      <div class="g-title">{{ item.title }}<span v-if="item.priority" class="g-badge" :class="item.priority">優先度 {{ prio[item.priority] }}</span><span v-if="crowd && ds" class="g-badge" :class="'ds-' + ds">{{ displayStatusLabel(ds) }}</span></div>
      <div v-if="item.brand || item.spec" class="g-mute">{{ [item.brand, item.spec].filter(Boolean).join('・') }}</div>
      <template v-if="masked"><div class="g-mute">驚喜模式中</div></template>
      <template v-else-if="crowd">
        <div class="g-bar" role="progressbar" :aria-valuenow="item.progress_percent ?? 0" aria-valuemin="0" aria-valuemax="100"><i :style="{ width: (item.progress_percent ?? 0) + '%' }" /></div>
        <div v-if="item.pledged_points != null" class="g-mute">已募 <b class="g-num">{{ fmtPts(item.pledged_points) }} / {{ fmtPts(item.target_points) }} 點</b><template v-if="ds === 'open'">・還差 {{ fmtPts(item.remaining_points) }} 點</template></div>
        <div v-else class="g-mute">目標 <b class="g-num">{{ fmtPts(item.target_points) }} 點</b></div>
        <div v-if="item.funding_deadline && ds === 'open'" class="g-mute">募集至 {{ fmtTime(item.funding_deadline) }}（台北時間）</div>
        <div v-if="item.contributors?.length" class="g-mute">{{ item.contributors.map((c: any) => `${c.display_name} ${fmtPts(c.points)} 點`).join('、') }} 已贊助</div>
      </template>
      <template v-else>
        <div class="g-bar" role="progressbar" :aria-valuenow="item.progress_percent" aria-valuemin="0" aria-valuemax="100"><i :style="{ width: item.progress_percent + '%' }" /></div>
        <div class="g-mute">已認領 <b class="g-num">{{ item.qty_claimed }} / {{ item.qty_needed }}</b><template v-if="mine">・你已認領 {{ mine.qty }} 件</template></div>
        <div v-if="item.claimers?.length" class="g-mute">{{ item.claimers.map((c: any) => c.display_name).join('、') }} 已認領</div>
      </template>
      <div class="g-actions">
        <span v-if="masked" class="g-mute">建立者不可認領自己的清單</span>
        <template v-else-if="crowd">
          <button v-if="ds === 'open' && !closed" class="g-btn sm js-claim" :disabled="!online" @click="$emit('fund')">用點數贊助</button>
          <span v-else-if="ds && ds !== 'open'" class="g-mute">{{ ds === 'expired' ? '募集已截止' : '已募集完成' }}</span>
        </template>
        <button v-else-if="mine && !closed" class="g-btn ghost sm js-claim" :disabled="!online" @click="$emit('claim')">修改我的認領</button>
        <span v-else-if="full" class="g-mute">已被認領完</span>
        <button v-else-if="!closed" class="g-btn sm js-claim" :disabled="!online" @click="$emit('claim')">我要送</button>
        <a v-if="item.product_url && !masked" :href="item.product_url" target="_blank" rel="noopener nofollow" class="g-link">商品連結</a>
      </div>
    </div>
  </li>
</template>
