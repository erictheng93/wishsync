import { beforeEach, describe, expect, it, vi } from 'vitest'
import { VIS_OPTIONS, getListAccess, isRestricted, setListAccess, visDesc } from '../app/utils/visibility'

describe('visibility', () => {
  beforeEach(() => {
    const m = new Map<string, string>()
    vi.stubGlobal('sessionStorage', { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => m.set(k, v), removeItem: (k: string) => m.delete(k) })
  })
  it('六種可見性皆有說明', () => {
    expect(VIS_OPTIONS.map(o => o.v)).toEqual(['public', 'link', 'friends', 'selected', 'password', 'private'])
    expect(visDesc('friends')).toContain('好友'); expect(visDesc('x')).toBe('')
  })
  it('只有 public / link 不需 client 重抓', () => {
    expect(['public', 'link'].some(isRestricted)).toBe(false)
    expect(['friends', 'selected', 'password', 'private'].every(isRestricted)).toBe(true)
  })
  it('權杖存取 ws_la_<slug>', () => {
    setListAccess('abc', 'tok'); expect(getListAccess('abc')).toBe('tok'); expect(sessionStorage.getItem('ws_la_abc')).toBe('tok')
    setListAccess('abc', null); expect(getListAccess('abc')).toBeNull()
  })
  it('儲存被停用時不拋錯', () => {
    vi.stubGlobal('sessionStorage', { getItem: () => { throw new Error('x') }, setItem: () => { throw new Error('x') }, removeItem: () => { throw new Error('x') } })
    expect(() => setListAccess('a', 't')).not.toThrow(); expect(getListAccess('a')).toBeNull()
  })
  it('連讀取 sessionStorage 屬性都丟例外（LINE 停用儲存）時不拋錯', () => {
    vi.unstubAllGlobals()
    const d = Object.getOwnPropertyDescriptor(globalThis, 'sessionStorage')
    Object.defineProperty(globalThis, 'sessionStorage', { configurable: true, get() { throw new DOMException('denied', 'SecurityError') } })
    try { expect(getListAccess('a')).toBeNull(); expect(() => setListAccess('a', 't')).not.toThrow() }
    finally { d ? Object.defineProperty(globalThis, 'sessionStorage', d) : delete (globalThis as any).sessionStorage }
  })
})
