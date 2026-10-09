// 創建者頁面為純客戶端驗證（cookie 在跨網域 SSR 取不到）
export default defineNuxtRouteMiddleware(async (to) => {
  if (import.meta.server) return
  const u = await useAuth().fetchMe()
  if (!u) return navigateTo({ path: '/login', query: { redirect: to.fullPath } })
})
