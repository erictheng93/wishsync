import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.stubGlobal('useRuntimeConfig', () => ({ public: { apiBase: '' } }))

function env(o: { ls?: 'ok' | 'throw', cookie?: boolean, ss?: 'ok' | 'throw' }) {
  const ls = new Map<string, string>(), ss = new Map<string, string>()
  let ck = ''
  const store = (m: Map<string, string>, mode?: string) => ({
    getItem: (k: string) => { if (mode === 'throw') throw new Error('denied'); return m.get(k) ?? null },
    setItem: (k: string, v: string) => { if (mode === 'throw') throw new Error('denied'); m.set(k, v) },
    removeItem: (k: string) => { m.delete(k) },
  })
  vi.stubGlobal('localStorage', store(ls, o.ls))
  vi.stubGlobal('sessionStorage', store(ss, o.ss))
  vi.stubGlobal('document', {
    get cookie() { return ck },
    set cookie(v: string) { if (!o.cookie) return; const [kv] = v.split(';'); ck = /Max-Age=0/.test(v) ? '' : kv },
  })
  return { ls, ss, get ck() { return ck } }
}

beforeEach(() => vi.resetModules())
const load = async () => await import('../app/composables/useGuest')

describe('guest token fallback chain', () => {
  it('localStorage ok -> persisted', async () => {
    const e = env({ ls: 'ok', cookie: true, ss: 'ok' }); const g = await load()
    expect(g.setGuestToken('t1')).toBe(true); expect(e.ls.get('ws_guest_token')).toBe('t1'); expect(g.getGuestToken()).toBe('t1')
  })
  it('localStorage throws -> cookie', async () => {
    env({ ls: 'throw', cookie: true, ss: 'ok' }); const g = await load()
    expect(g.setGuestToken('t2')).toBe(true); expect(g.getGuestToken()).toBe('t2')
  })
  it('localStorage + cookie fail -> sessionStorage, not persisted', async () => {
    const e = env({ ls: 'throw', cookie: false, ss: 'ok' }); const g = await load()
    expect(g.setGuestToken('t3')).toBe(false); expect(e.ss.get('ws_guest_token')).toBe('t3'); expect(g.getGuestToken()).toBe('t3')
  })
  it('everything fails -> memory only', async () => {
    env({ ls: 'throw', cookie: false, ss: 'throw' }); const g = await load()
    expect(g.setGuestToken('t4')).toBe(false); expect(g.getGuestToken()).toBe('t4')
  })
  it('clearing removes the token everywhere', async () => {
    env({ ls: 'ok', cookie: true, ss: 'ok' }); const g = await load()
    g.setGuestToken('t5'); g.setGuestToken(null); expect(g.getGuestToken()).toBeNull()
  })
})
