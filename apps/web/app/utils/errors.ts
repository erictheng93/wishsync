// 訪客端錯誤碼 → 使用者文案（純函式，可單元測試）
export function reportErrMsg(e: { code?: string, status?: number, detail?: string, data?: any }): { msg: string, resetCaptcha: boolean } {
  if (e.code === 'RATE_LIMITED' || e.status === 429) return { msg: '檢舉太頻繁，請稍後再試', resetCaptcha: false }
  const turnstile = e.code === 'TURNSTILE_FAILED' || (e.data?.errors ?? []).some((x: any) => x.pointer === '/turnstile_token')
  if (turnstile) return { msg: '人機驗證未通過或已過期，請重新驗證後再送出', resetCaptcha: true }
  if (e.code === 'NETWORK') return { msg: '網路不穩，再試一次', resetCaptcha: true }
  return { msg: e.detail || '發生錯誤，請稍後再試', resetCaptcha: true }
}
