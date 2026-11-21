// 诊断信息: what went wrong in this browser lately, for 设置 → 诊断信息,
// where it can be read and copied (on a phone too, without developer tools).
// Errors nobody caught (scripts, promises, rendering a component), and the
// failures the code handles quietly but someone looking into a problem
// wants to know about (storage refusing to save, syncing).
//
// Kept in this browser only (localStorage), never sent anywhere; the same
// problem again only counts up.

import { ref, type App } from 'vue'

export interface Problem {
  /** Last time, ms. */
  at: number
  where: string
  message: string
  count: number
}

const KEY = 'diagnostics'
const MAX = 50

function load(): Problem[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    return Array.isArray(v) ? v.filter((p) => typeof p?.message === 'string').slice(0, MAX) : []
  } catch {
    return []
  }
}

export const problems = ref<Problem[]>(load())

function save() {
  try {
    localStorage.setItem(KEY, JSON.stringify(problems.value))
  } catch {
    // Storage is the likely problem itself; kept for this visit.
  }
}

function describe(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}`.slice(0, 500)
  if (typeof e === 'string') return e.slice(0, 500)
  try {
    return JSON.stringify(e).slice(0, 500)
  } catch {
    return String(e).slice(0, 500)
  }
}

/** Notes a problem (and logs it to the console, as before). */
export function noteProblem(where: string, e: unknown) {
  console.warn(`[${where}]`, e)
  const message = describe(e)
  const same = problems.value.find((p) => p.where === where && p.message === message)
  const rest = problems.value.filter((p) => p !== same)
  problems.value = [{ at: Date.now(), where, message, count: (same?.count ?? 0) + 1 }, ...rest].slice(0, MAX)
  save()
}

export function clearProblems() {
  problems.value = []
  save()
}

// Harmless browser noise: a script from another site (no details given), and
// ResizeObserver telling that a layout took two frames.
const NOISE = /^(Script error\.?|ResizeObserver loop)/

window.addEventListener('error', (e) => {
  const message = e.error ?? e.message
  if (!NOISE.test(describe(message))) noteProblem('页面脚本', message)
})
window.addEventListener('unhandledrejection', (e) => noteProblem('没有处理的异步错误', e.reason))

/** Errors while showing a component: noted, and logged as Vue would without a handler. */
export function watchComponents(app: App) {
  app.config.errorHandler = (err, _instance, info) => {
    console.error(err)
    noteProblem(`界面（${info}）`, err)
  }
}
