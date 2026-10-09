import { describe, expect, it } from 'vitest'
import { apiErrMsg } from '../app/utils/errors'

describe('apiErrMsg（依 code / status，不依賴 detail）', () => {
  it.each([
    [{ status: 401, code: 'UNAUTHORIZED' }, '身分已失效'],
    [{ status: 404, code: 'NOT_FOUND' }, '這個品項已不存在'],
    [{ status: 410, code: 'WISHLIST_REMOVED' }, '這份清單已被下架'],
    [{ status: 429 }, '稍後再試'],
    [{ status: 503 }, '系統維護中'],
    [{ status: 0, code: 'NETWORK' }, '網路不穩'],
  ])('%j', (e, part) => expect(apiErrMsg(e)).toContain(part))
  it('code 優先於 detail；沒有 detail 也有文案', () => {
    expect(apiErrMsg({ status: 410, code: 'WISHLIST_REMOVED', detail: '' })).toBe('這份清單已被下架')
    expect(apiErrMsg({ status: 404, code: 'CLAIM_NOT_FOUND' })).toBe('這筆認領已不存在')
  })
  it('驗證類錯誤沿用 detail；未知 5xx 才用泛用文案', () => {
    expect(apiErrMsg({ status: 422, code: 'VALIDATION_FAILED', detail: '暱稱需為 1–30 字' })).toBe('暱稱需為 1–30 字')
    expect(apiErrMsg({ status: 500 })).toContain('系統暫時發生問題')
    expect(apiErrMsg({})).toBe('發生錯誤，請稍後再試')
  })
})
