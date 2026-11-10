// The reader pane: on a wide screen a blog's page (a post's original, its
// comments, the blog's homepage) opens in the right part of this page instead
// of a new tab, so the feed stays in view on the left. Sites that forbid
// being shown in a frame, and links opened with a modifier key or the middle
// button, still get a new tab.

import { computed, ref, watch } from 'vue'
import type { BlogRef, PostCard } from './types'

export interface ReaderPage {
  url: string
  title: string
  blog: Pick<BlogRef, 'id' | 'name' | 'icon' | 'framable'>
  /** The post, when the page is one: the pane offers like and save. */
  post?: PostCard
}

/** Window width from which the page splits. */
const MIN_WIDTH = 1100
/** The left part never gets narrower than this. */
const MIN_LEFT = 480
const SHARE_KEY = 'reader-share'
/** Blogs a reader found would not show in the pane (this browser only). */
const BLOCKED_KEY = 'reader-blocked'

export const readerPage = ref<ReaderPage | null>(null)

const media = matchMedia(`(min-width: ${MIN_WIDTH}px)`)
const wide = ref(media.matches)
media.addEventListener('change', (e) => (wide.value = e.matches))

/** The pane shows only while the window is wide enough. */
export const readerOpen = computed(() => !!readerPage.value && wide.value)

function loadShare(): number {
  try {
    const v = Number(localStorage.getItem(SHARE_KEY))
    return v >= 0.3 && v <= 0.7 ? v : 0.5
  } catch {
    return 0.5
  }
}

/** Share of the window's width the pane takes. */
export const readerShare = ref(loadShare())

export function setReaderShare(share: number, save: boolean) {
  const max = Math.min(0.7, (window.innerWidth - MIN_LEFT) / window.innerWidth)
  readerShare.value = Math.min(max, Math.max(0.3, share))
  if (!save) return
  try {
    localStorage.setItem(SHARE_KEY, String(readerShare.value))
  } catch {
    // Kept for this visit only.
  }
}

function blocked(): number[] {
  try {
    const v = JSON.parse(localStorage.getItem(BLOCKED_KEY) ?? '[]')
    return Array.isArray(v) ? v.filter(Number.isInteger) : []
  } catch {
    return []
  }
}

/** Remembers that this blog's pages would not show in the pane. */
export function markBlocked(blogId: number) {
  try {
    localStorage.setItem(BLOCKED_KEY, JSON.stringify([blogId, ...blocked().filter((x) => x !== blogId)].slice(0, 500)))
  } catch {
    // Applies to this visit only.
  }
}

function canShow(page: ReaderPage): boolean {
  if (!wide.value || page.blog.framable === false || blocked().includes(page.blog.id)) return false
  // A page served over https cannot show an http one.
  return !(location.protocol === 'https:' && page.url.startsWith('http:'))
}

/**
 * Opens `page` in the pane when it can show there. Returns false when the
 * link should do what links do (open a new tab); call it from the link's
 * click handler and prevent the default only when it returns true.
 */
export function openInReader(page: ReaderPage, e?: MouseEvent): boolean {
  if (e && (e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey)) return false
  if (!canShow(page)) return false
  readerPage.value = page
  if (readerAhead.value === page.url) readerAhead.value = null
  return true
}

export function closeReader() {
  readerPage.value = null
}

/**
 * A page that will probably open in the pane next, loading out of sight so
 * it shows at once. Only on a clear sign (the pointer resting on its link),
 * since loading it counts as a visit on the author's site.
 */
export const readerAhead = ref<string | null>(null)

/** How long the pointer rests on a link before its page loads ahead. */
const REST_MS = 150
/** A page loaded ahead and not opened is let go after this. */
const AHEAD_MS = 60_000
let restTimer = 0
let aheadTimer = 0

/** The pointer reached (or the keyboard focused) a link to `page`. */
export function readerIntent(page: ReaderPage, delay = REST_MS) {
  clearTimeout(restTimer)
  if (!canShow(page) || readerPage.value?.url === page.url) return
  restTimer = window.setTimeout(() => {
    readerAhead.value = page.url
    clearTimeout(aheadTimer)
    aheadTimer = window.setTimeout(() => (readerAhead.value = null), AHEAD_MS)
  }, delay)
}

/** The pointer left the link before resting on it. */
export function readerIntentEnd() {
  clearTimeout(restTimer)
}

const warmed = new Set<string>()

/**
 * Looks up and connects to a site whose page may be opened soon (a post's
 * details are showing): no request for the page itself.
 */
export function warmUp(url: string) {
  let origin: string
  try {
    origin = new URL(url).origin
  } catch {
    return
  }
  if (warmed.has(origin) || warmed.size >= 50) return
  warmed.add(origin)
  for (const rel of ['dns-prefetch', 'preconnect']) {
    const link = document.createElement('link')
    link.rel = rel
    link.href = origin
    document.head.append(link)
  }
}

// The left part is laid out as if the window ended where the pane begins.
watch(
  [readerOpen, readerShare],
  ([open, share]) => {
    document.documentElement.style.setProperty('--pane-w', open ? `${(share * 100).toFixed(2)}vw` : '0px')
  },
  { immediate: true },
)
