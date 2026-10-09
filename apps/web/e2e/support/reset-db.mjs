// 重建 e2e 專用資料庫（與開發用的 wishsync 完全分開）。由 playwright.config 的 API webServer 在啟動 API 前執行，
// API 啟動時會自動套用 migrations。
import { psql } from './psql.mjs'

const db = 'wishsync_e2e'
psql('postgres', `DROP DATABASE IF EXISTS ${db} WITH (FORCE)`)
psql('postgres', `CREATE DATABASE ${db}`)
console.log(`[e2e] 已重建資料庫 ${db}`)
