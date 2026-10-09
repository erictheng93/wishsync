export default defineNuxtRouteMiddleware(async (to) => {
  if (import.meta.server) return
  const u = await useAuth().fetchMe()
  if (!u) return navigateTo({ path: '/login', query: { redirect: to.fullPath } })
  if (!u.is_staff) return abortNavigation(createError({ statusCode: 403, statusMessage: '這個頁面僅限營運人員' }))
})
