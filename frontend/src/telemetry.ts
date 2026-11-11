// 统计: what readers do here, counted on the server with nobody in it
// (backend/src/stats.rs, or the Worker for the static site), for heat, for
// 推荐 when a reader wants others' reading to count, and for 大家在看.
//
// Sent unless turned off (设置 → 统计与热度): a visit a day, time on the site
// and on each post, opening and reading posts, likes, saves, follows,
// searches, and once a day the settings in use. With a random id of this
// browser's own (not the sync code, not shared with other devices), only so
// the server can count each visitor, and each own feed's readers, once; it
// keeps it hashed, and no IP. Sent in batches: half a minute after something
// happened, and on leaving (with the time spent).

import { watch } from 'vue'
import { options } from './options'
import { loadCapabilities } from './server'

interface Event {
  /** visit | dwell | dwell_post | open | read | like | save | follow | search | setting */
  t: string
  k?: string
  n?: number
}

const ID_KEY = 'stats:id'
const DAY_KEY = 'stats:day'
const FLUSH_MS = 30_000

function stored(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}
function keep(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // This visit only.
  }
}

/** This browser's random id: 24 letters and digits, made once. */
export function statsId(): string {
  let id = stored(ID_KEY)
  if (!id || !/^[A-Za-z0-9]{16,64}$/.test(id)) {
    const abc = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'
    id = [...crypto.getRandomValues(new Uint8Array(24))].map((b) => abc[b % abc.length]).join('')
    keep(ID_KEY, id)
  }
  return id
}

export const sendingStats = () => options.value.sendStats

const queue: Event[] = []
let timer: ReturnType<typeof setTimeout> | undefined

export function track(t: string, k?: string, n?: number) {
  if (!sendingStats()) return
  queue.push({ t, ...(k !== undefined ? { k } : {}), ...(n !== undefined ? { n } : {}) })
  if (queue.length >= 100) void flush()
  else timer ??= setTimeout(() => void flush(), FLUSH_MS)
}

/** Time on the site, counted while the page shows. */
let visibleSince = document.visibilityState === 'visible' ? Date.now() : 0
function takeDwell() {
  if (!visibleSince) return
  const secs = Math.round((Date.now() - visibleSince) / 1000)
  visibleSince = document.visibilityState === 'visible' ? Date.now() : 0
  if (secs > 0 && sendingStats()) queue.push({ t: 'dwell', n: Math.min(secs, 3600) })
}

async function flush(leaving = false) {
  clearTimeout(timer)
  timer = undefined
  takeDwell()
  if (!queue.length || !sendingStats() || !(await loadCapabilities()).stats) {
    queue.length = 0
    return
  }
  const body = JSON.stringify({ id: statsId(), events: queue.splice(0, 200) })
  if (leaving && navigator.sendBeacon?.('/api/events', new Blob([body], { type: 'application/json' }))) return
  fetch('/api/events', { method: 'POST', headers: { 'content-type': 'application/json' }, body, keepalive: true }).catch(
    () => {},
  )
}

document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'hidden') void flush(true)
  else visibleSince = Date.now()
})
window.addEventListener('pagehide', () => void flush(true))

/** Once a day: that this browser came, and which settings it uses. */
function daily() {
  const today = new Date().toISOString().slice(0, 10)
  if (!sendingStats() || stored(DAY_KEY) === today) return
  keep(DAY_KEY, today)
  track('visit')
  const o = options.value
  const settings: Record<string, string | number | boolean> = {
    text: o.textSize,
    pins: o.maxPinned,
    nsfw: o.showNsfw,
    others: o.useOthers,
    heat: o.showHeat,
    blocks: o.blockBlogs.length + o.blockHosts.length + o.blockWords.length > 0,
  }
  for (const [k, v] of Object.entries(settings)) track('setting', `${k}=${v}`)
}
daily()
setInterval(daily, 3600_000)

// Turned off: what is waiting is dropped.
watch(sendingStats, (on) => {
  if (!on) queue.length = 0
})

// Time on one post: from opening it until it closes or the next one opens.
let reading: { url: string; since: number } | null = null
export function postOpened(url: string) {
  postClosed()
  reading = { url, since: Date.now() }
  track('open', url)
}
export function postClosed() {
  if (!reading) return
  const secs = Math.round((Date.now() - reading.since) / 1000)
  if (secs >= 3) track('dwell_post', reading.url, Math.min(secs, 3600))
  reading = null
}
