// 只在 production 註冊 service worker；dev 不註冊以免干擾開發。不支援（如部分內建瀏覽器）或失敗時靜默略過。
export default defineNuxtPlugin(() => {
  if (import.meta.dev || !('serviceWorker' in navigator)) return
  const reg = () => navigator.serviceWorker.register('/sw.js').catch(() => {})
  // 水合可能發生在 load 之後，此時 load 事件不會再來
  if (document.readyState === 'complete') reg()
  else addEventListener('load', reg)
})
