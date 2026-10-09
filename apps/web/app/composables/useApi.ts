// 創建者端 API 客戶端：cookie（ws_session）驗證、RFC 9457 錯誤正規化
export class CreatorApiError extends Error {
  constructor(public status: number, public code: string, public detail: string, public errors: any[] = [], public body: any = {}) { super(detail) }
}

export function errMsg(e: any): string {
  if (!(e instanceof CreatorApiError)) return '發生未知錯誤，請稍後再試'
  if (e.code === 'READ_ONLY_MODE') return '系統維護中，稍後再試'
  if (e.code === 'NETWORK') return '連線失敗，請檢查網路後再試'
  if (e.errors.length) return e.errors.map((x: any) => x.detail).join('；')
  return e.detail
}

export function useApi() {
  const router = useRouter()
  const base = useRuntimeConfig().public.apiBase.replace(/\/$/, '') + '/api/v1'
  async function api<T = any>(path: string, opts: any = {}): Promise<T> {
    const { noRedirect, ...rest } = opts
    try {
      return await $fetch<T>(path, { baseURL: base, credentials: 'include', ...rest })
    } catch (e: any) {
      const status = e.statusCode || e.status || 0
      const d = e.data && typeof e.data === 'object' ? e.data : {}
      const err = new CreatorApiError(status, d.code || (status ? 'HTTP_' + status : 'NETWORK'), d.detail || '連線失敗，請稍後再試', d.errors || [], d)
      if (err.code === 'UNAUTHORIZED' && !noRedirect && import.meta.client) {
        useState('auth-user').value = null
        await navigateTo({ path: '/login', query: { redirect: router.currentRoute.value.fullPath } })
      }
      throw err
    }
  }
  return { api, base }
}
