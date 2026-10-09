import type { Page } from '@playwright/test'
/** SSR 的按鈕在 hydrate 前點了不會有作用；goto 之後先等 Nuxt hydrate 完 */
export const hydrated = (page: Page) => page.waitForFunction(() => (window as any).useNuxtApp?.().isHydrating === false)
