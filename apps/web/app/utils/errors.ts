// 訪客端錯誤碼 → 使用者文案（純函式，可單元測試）
export function reportErrMsg(e: { code?: string, status?: number, detail?: string, data?: any }): { msg: string, resetCaptcha: boolean } {
  if (e.code === 'RATE_LIMITED' || e.status === 429) return { msg: '檢舉太頻繁，請稍後再試', resetCaptcha: false }
  const turnstile = e.code === 'TURNSTILE_FAILED' || (e.data?.errors ?? []).some((x: any) => x.pointer === '/turnstile_token')
  if (turnstile) return { msg: '人機驗證未通過或已過期，請重新驗證後再送出', resetCaptcha: true }
  if (e.code === 'NETWORK') return { msg: '網路不穩，再試一次', resetCaptcha: true }
  return { msg: apiErrMsg(e), resetCaptcha: true }
}

// 一般 API 錯誤 → 使用者文案：先看 code / HTTP 狀態（不依賴後端是否有補 detail），其餘才用 detail
export function apiErrMsg(e: { code?: string, status?: number, detail?: string }): string {
  const s = e.status ?? 0
  if (e.code === 'NETWORK' || e.status === 0) return '網路不穩，再試一次'
  if (e.code === 'WISHLIST_REMOVED' || s === 410) return '這份清單已被下架'
  if (e.code === 'UNAUTHORIZED' || s === 401) return '你的身分已失效，請重新整理頁面後再試；若要找回之前的認領，請用確認信中的連結'
  if (e.code === 'HANDLE_TAKEN') return '這個帳號代號已被使用，換一個試試'
  if (e.code === 'SELF_INVITE') return '這是你自己的邀請連結，請分享給朋友'
  if (e.code === 'LOGIN_REQUIRED') return '請先登入才能查看這份清單'
  if (e.code === 'FRIENDS_ONLY') return '這份清單只開放給擁有者的好友'
  if (e.code === 'NOT_ALLOWED') return '擁有者沒有開放你查看這份清單'
  if (e.code === 'PASSWORD_REQUIRED') return '這份清單需要密碼'
  if (e.code === 'WRONG_PASSWORD') return '密碼不正確，請再試一次'
  if (e.code === 'NOT_FRIEND') return '只能選擇已成為好友的人'
  if (e.code === 'RATE_LIMITED' || s === 429) return '操作太頻繁，請稍後再試'
  if (s === 503) return '系統維護中，請稍後再試'
  if (s === 404) return e.code === 'CLAIM_NOT_FOUND' ? '這筆認領已不存在' : '這個品項已不存在，請重新整理頁面'
  if (e.detail && s < 500) return e.detail
  if (s >= 500) return '系統暫時發生問題，請稍後再試'
  return e.detail || '發生錯誤，請稍後再試'
}

// 個人頁代號：小寫英數與底線，3–30 字
export const isValidHandle = (h: string) => /^[a-z0-9_]{3,30}$/.test(h)
