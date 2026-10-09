import { describe, expect, it } from 'vitest'
import { reportErrMsg } from '../app/utils/errors'

describe('reportErrMsg', () => {
  it('rate limit does not reset captcha', () => {
    expect(reportErrMsg({ code: 'RATE_LIMITED' })).toEqual({ msg: '檢舉太頻繁，請稍後再試', resetCaptcha: false })
    expect(reportErrMsg({ status: 429 }).resetCaptcha).toBe(false)
  })
  it('turnstile failures reset widget (token is single-use)', () => {
    expect(reportErrMsg({ code: 'TURNSTILE_FAILED' })).toMatchObject({ resetCaptcha: true, msg: expect.stringContaining('人機驗證') })
    expect(reportErrMsg({ code: 'VALIDATION_FAILED', data: { errors: [{ pointer: '/turnstile_token' }] } }).msg).toContain('人機驗證')
  })
  it('falls back to detail', () => {
    expect(reportErrMsg({ code: 'X', detail: '壞了' }).msg).toBe('壞了')
    expect(reportErrMsg({}).msg).toBe('發生錯誤，請稍後再試')
  })
})
