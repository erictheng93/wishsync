export function useAuth() {
  const user = useState<any>('auth-user', () => null)
  const loaded = useState<boolean>('auth-loaded', () => false)
  const { api } = useApi()
  async function fetchMe(force = false) {
    if (loaded.value && !force) return user.value
    try { user.value = await api('/me', { noRedirect: true }) } catch { user.value = null }
    loaded.value = true
    return user.value
  }
  async function logout() {
    try { await api('/auth/logout', { method: 'POST', noRedirect: true }) } catch {}
    user.value = null
    await navigateTo('/login')
  }
  const post = (path: string, body: any) => api(path, { method: 'POST', body, noRedirect: true, credentials: 'include' })
  // 成功回 session cookie；重抓 /me 讓全域 user 就緒
  async function session(path: string, body: any) { await post(path, body); return fetchMe(true) }
  const login = (email: string, password: string) => session('/auth/login', { email, password })
  const register = (email: string, password: string, display_name: string) => post('/auth/register', { email, password, display_name })
  const verifyRegister = (email: string, code: string) => session('/auth/register/verify', { email, code })
  const requestReset = (email: string) => post('/auth/password/reset/request', { email })
  const confirmReset = (email: string, code: string, new_password: string) => post('/auth/password/reset/confirm', { email, code, new_password })
  return { user, fetchMe, logout, login, register, verifyRegister, requestReset, confirmReset }
}
