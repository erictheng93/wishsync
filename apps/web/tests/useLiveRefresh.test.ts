import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createLiveRefresh, liveLabel } from '../app/composables/useLiveRefresh'

class FakeES {
  static last: FakeES
  readyState = 0; onopen: any; onerror: any; closed = false
  constructor(public url: string) { FakeES.last = this }
  addEventListener() {}
  close() { this.closed = true }
}
let vis: any, hidden = false
beforeEach(() => {
  vi.useFakeTimers()
  vis = new EventTarget()
  vi.stubGlobal('EventSource', FakeES)
  Object.defineProperty(vis, 'visibilityState', { get: () => (hidden ? 'hidden' : 'visible') })
  vi.stubGlobal('document', vis)
  hidden = false
})
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals() })

function setup() {
  const refresh = vi.fn(), modes: string[] = []
  const l = createLiveRefresh({ url: 'u', refresh, onEvent: vi.fn(), onMode: m => modes.push(m) })
  l.start()
  return { l, refresh, modes }
}

describe('createLiveRefresh', () => {
  it('goes live on open and does not poll', () => {
    const { refresh, modes } = setup()
    FakeES.last.onopen()
    vi.advanceTimersByTime(60000)
    expect(modes).toEqual(['live']); expect(refresh).not.toHaveBeenCalled()
  })
  it('CLOSED error: refresh once, then poll every 15s', () => {
    const { refresh, modes } = setup()
    FakeES.last.readyState = 2; FakeES.last.onerror()
    expect(modes).toEqual(['polling']); expect(refresh).toHaveBeenCalledTimes(1)
    vi.advanceTimersByTime(15000); expect(refresh).toHaveBeenCalledTimes(2)
  })
  it('3 reconnect failures switch to polling; 2 do not', () => {
    const { modes } = setup()
    FakeES.last.onerror(); FakeES.last.onerror()
    expect(modes).toEqual([])
    FakeES.last.onerror(); expect(modes).toEqual(['polling'])
  })
  it('no connection within 30s switches to polling', () => {
    const { modes } = setup()
    vi.advanceTimersByTime(30000); expect(modes).toEqual(['polling'])
  })
  it('SSE recovery stops polling', () => {
    const { refresh, modes } = setup()
    vi.advanceTimersByTime(30000); FakeES.last.onopen()
    refresh.mockClear(); vi.advanceTimersByTime(60000)
    expect(modes).toEqual(['polling', 'live']); expect(refresh).not.toHaveBeenCalled()
  })
  it('pauses while hidden, refreshes immediately on return', () => {
    const { refresh } = setup()
    vi.advanceTimersByTime(30000); hidden = true
    vi.advanceTimersByTime(45000); expect(refresh).not.toHaveBeenCalled()
    hidden = false; vis.dispatchEvent(new Event('visibilitychange')); expect(refresh).toHaveBeenCalledTimes(1)
  })
  it('no EventSource support polls directly; stop() cleans up', () => {
    vi.stubGlobal('EventSource', undefined)
    const { l, refresh, modes } = setup()
    expect(modes).toEqual(['polling'])
    l.stop(); vi.advanceTimersByTime(60000); expect(refresh).not.toHaveBeenCalled()
  })
  it('labels', () => {
    expect([liveLabel('live'), liveLabel('polling'), liveLabel('connecting')]).toEqual(['即時更新中', '定時更新', '連線中…'])
  })
})
