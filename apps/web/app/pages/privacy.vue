<script setup lang="ts">
import '~/assets/guest.css'
useSeoMeta({ title: '隱私權政策' })
</script>
<template>
  <GuestTop />
  <main class="g-wrap g-doc">
    <h1>隱私權政策</h1>
    <p class="g-mute">版本：草案（正式內容上線前請由法務確認）</p>
    <h2 id="collect">個資蒐集告知（認領時）</h2>
    <ul>
      <li>蒐集項目：暱稱（必填）、聯絡方式（選填）、Email（選填）、留言（選填）。</li>
      <li>目的：讓清單建立者知道誰認領了哪些品項並與你聯繫；Email 用於寄送認領確認信與「管理我的認領」連結。</li>
      <li>揭露對象：暱稱與聯絡方式僅清單建立者可見；Email 不會顯示給其他訪客與建立者。若建立者開啟「顯示認領者」且非驚喜鎖定期間，其他訪客可看到你的暱稱。</li>
      <li>不填寫不影響你認領，但留 Email 才能在換手機後找回認領。</li>
    </ul>
    <h2>裝置儲存</h2>
    <p>為了在「我的認領」識別你，我們會在你的瀏覽器儲存一組隨機識別碼（localStorage 與 cookie）與你上次使用的暱稱。LINE 內建瀏覽器可能清除這些資料，建議用預設瀏覽器開啟。</p>
    <h2>你的權利</h2>
    <p>你可在「我的認領」修改暱稱，或選擇「刪除我的暱稱與聯絡方式」：我們會清除暱稱、聯絡方式與 Email，認領紀錄保留為「已刪除的訪客」供建立者對帳。信件頁尾可一鍵退訂通知信。</p>
    <h2>檢舉</h2>
    <p>檢舉採人工審核，不會告知建立者檢舉人身分；為防濫用會使用 Cloudflare Turnstile 與依 IP 限流。</p>
  </main>
</template>
