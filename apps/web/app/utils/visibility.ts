// 清單可見性：選項文案（同時作為分享時「誰能打開」的提示）+ 密碼清單權杖（sessionStorage ws_la_<slug>）
export const VIS_OPTIONS = [
  { v: 'public', label: '公開', desc: '任何人都能看；會列在你的個人頁。' },
  { v: 'link', label: '知道連結', desc: '只有拿到連結的人能開，不會列在個人頁。' },
  { v: 'friends', label: '僅好友', desc: '只有你的好友能開，對方需要登入。' },
  { v: 'selected', label: '指定好友', desc: '只有你勾選的好友能開，對方需要登入。' },
  { v: 'password', label: '密碼保護', desc: '拿到連結且知道密碼的人才能開。' },
  { v: 'private', label: '私人', desc: '只有你自己看得到，朋友打開會顯示找不到。' },
] as const
export type Visibility = typeof VIS_OPTIONS[number]['v']
export const visDesc = (v: string) => VIS_OPTIONS.find(o => o.v === v)?.desc ?? ''

// 受限清單（SSR 沒有 cookie 會拿到 403）：要等 client 帶憑證重抓
export const isRestricted = (v?: string) => !!v && !['public', 'link'].includes(v)

const sk = (slug: string) => `ws_la_${slug}`
export function getListAccess(slug: string): string | null {
  if (typeof sessionStorage === 'undefined') return null
  try { return sessionStorage.getItem(sk(slug)) } catch { return null }
}
export function setListAccess(slug: string, token: string | null) {
  try { token ? sessionStorage.setItem(sk(slug), token) : sessionStorage.removeItem(sk(slug)) } catch { /* 無痕/停用儲存：本次頁面重載後需重輸入密碼 */ }
}
