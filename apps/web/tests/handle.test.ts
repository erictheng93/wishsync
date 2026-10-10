import { describe, expect, it } from 'vitest'
import { isValidHandle } from '../app/utils/errors'

describe('isValidHandle', () => {
  it('accepts a-z 0-9 _ of 3-30 chars', () => {
    expect(isValidHandle('abc')).toBe(true)
    expect(isValidHandle('a_1'.repeat(10))).toBe(true)
  })
  it('rejects short, long, uppercase, symbols', () => {
    for (const h of ['ab', 'a'.repeat(31), 'Abc', 'a-b', 'a b', '中文中', ''])
      expect(isValidHandle(h)).toBe(false)
  })
})
