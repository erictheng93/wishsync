import { defineConfig } from 'vitest/config'
// 這些測試只涵蓋純邏輯；useGuest 以 import.meta.client 判斷環境，於此注入為 true
export default defineConfig({ define: { 'import.meta.client': true }, test: { include: ['tests/**/*.test.ts'] } })
