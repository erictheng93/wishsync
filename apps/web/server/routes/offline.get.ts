// F-22：node-server preset（E2E）沒有 Cloudflare Pages 的「foo.html → /foo」靜態對應，/offline 會 404、SW 預先快取失敗。
// 這條路由回傳同一份 public/offline.html。Cloudflare Pages 上 public 靜態資產優先於 Functions（_routes.json 排除），所以兩者不衝突。
// @ts-expect-error nitro 的 raw: 前綴（建置時內嵌檔案文字），無型別宣告
import html from 'raw:../../public/offline.html'

export default defineEventHandler((event) => {
  setHeader(event, 'content-type', 'text/html; charset=utf-8')
  setHeader(event, 'cache-control', 'no-cache')
  return html
})
