// 不新增 npm 依賴的 SQL 執行器：優先用本機 psql（CI），否則走 docker compose 的 db 容器（本機開發）。
import { execFileSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../../..')
const has = (cmd) => { try { execFileSync(cmd, ['--version'], { stdio: 'ignore' }); return true } catch { return false } }
const useLocal = has('psql')

/** 對指定資料庫執行 SQL，回傳 stdout（-tA：無表頭、無對齊） */
export function psql(db, sql) {
  const base = ['-v', 'ON_ERROR_STOP=1', '-tA', '-c', sql]
  if (useLocal) {
    const pw = process.env.E2E_PGPASSWORD ?? 'wishsync'
    return execFileSync('psql', [`postgres://wishsync:${pw}@localhost:5432/${db}`, ...base], { encoding: 'utf8' })
  }
  return execFileSync('docker', ['compose', 'exec', '-T', 'db', 'psql', '-U', 'wishsync', '-d', db, ...base], { cwd: root, encoding: 'utf8' })
}
