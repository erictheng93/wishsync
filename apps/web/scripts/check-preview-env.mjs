// 預覽部署（測試用）：必須指定要代理的後端網址，且不得是正式網域以外的非 https。
const t = process.env.NUXT_API_PROXY
if (!t || !/^https:\/\/[^/\s]+$/.test(t.replace(/\/$/, ''))) {
  console.error('\n✘ 預覽部署需要 NUXT_API_PROXY=https://<後端網址>（例如快速通道網址）\n')
  process.exit(1)
}
