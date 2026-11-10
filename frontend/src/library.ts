// What this reader liked, saved, opened, and was shown (刷到过: cards that
// stayed on screen, opened or not), kept in this browser (localStorage; there
// are no accounts). Each entry keeps a copy of the card, so the lists still
// show posts that have since left the site's feed, and can be searched
// without the server. Backups: see backup.ts.

import { computed, readonly, ref, type Ref } from 'vue'
import type { PostCard } from './types'

export type ListName = 'likes' | 'saves' | 'history' | 'seen'

export interface Entry {
  post: PostCard
  /** When it was added (for 刷到过: last shown), ms since the epoch. */
  at: number
  /** 刷到过 only: how many times it was shown, and where last. */
  count?: number
  from?: string
}

export const LIST_NAMES: ListName[] = ['saves', 'likes', 'history', 'seen']
const LIMITS: Record<ListName, number> = { likes: 1000, saves: 2000, history: 500, seen: 1000 }
const key = (name: ListName) => `library:${name}`
const SETTINGS_KEY = 'library:settings'

/**
 * The card without what goes stale or is not needed to show and search it;
 * `compact` keeps less, for the long 刷到过 list.
 */
function snapshot(p: PostCard, compact = false): PostCard {
  return {
    id: p.id,
    title: p.title,
    url: p.url,
    excerpt: p.excerpt.slice(0, compact ? 100 : 280),
    cover: p.cover,
    author: compact ? null : p.author,
    tags: p.tags.slice(0, compact ? 3 : 8),
    auto_tags: compact ? [] : (p.auto_tags ?? []).slice(0, 4),
    category: p.category,
    published_at: p.published_at,
    sort_at: p.sort_at,
    undated: p.undated,
    has_full_text: false,
    comment_count: null,
    comments_url: null,
    blog: { id: p.blog.id, name: p.blog.name, homepage: p.blog.homepage, icon: p.blog.icon, kind: p.blog.kind },
  }
}

/** Accepts entries from storage or a backup file, dropping anything malformed. */
export function cleanEntries(v: unknown): Entry[] {
  if (!Array.isArray(v)) return []
  const out: Entry[] = []
  for (const e of v) {
    const p = e?.post
    if (!Number.isInteger(p?.id) || typeof p.title !== 'string' || typeof p.url !== 'string') continue
    if (!Number.isInteger(p.blog?.id) || typeof p.blog.name !== 'string') continue
    out.push({
      at: Number.isFinite(e.at) ? e.at : 0,
      count: Number.isInteger(e.count) ? e.count : undefined,
      from: typeof e.from === 'string' ? e.from.slice(0, 40) : undefined,
      post: {
        ...p,
        excerpt: typeof p.excerpt === 'string' ? p.excerpt : '',
        tags: Array.isArray(p.tags) ? p.tags.filter((t: unknown) => typeof t === 'string') : [],
        category: p.category?.id ? p.category : { id: 'other', label: '其他' },
        sort_at: Number.isFinite(p.sort_at) ? p.sort_at : 0,
      },
    })
  }
  return out
}

function load(name: ListName): Entry[] {
  try {
    return cleanEntries(JSON.parse(localStorage.getItem(key(name)) ?? '[]')).slice(0, LIMITS[name])
  } catch {
    return []
  }
}

function store(name: ListName, entries: Entry[]) {
  try {
    localStorage.setItem(key(name), JSON.stringify(entries))
  } catch {
    // Storage full or blocked. Make room by shortening the least precious
    // lists (刷到过, then the history), once.
    for (const [victim, keep] of [['seen', 200], ['history', 50]] as const) {
      if (name === victim || lists[victim].value.length <= keep) continue
      lists[victim].value = lists[victim].value.slice(0, keep)
      try {
        localStorage.setItem(key(victim), JSON.stringify(lists[victim].value))
        localStorage.setItem(key(name), JSON.stringify(entries))
        return
      } catch {
        // Try the next one.
      }
    }
    // Kept for this visit only.
  }
}

const lists: Record<ListName, Ref<Entry[]>> = {
  likes: ref(load('likes')),
  saves: ref(load('saves')),
  history: ref(load('history')),
  seen: ref(load('seen')),
}

interface Settings {
  /** Record opened posts in the history. */
  history: boolean
  /** Record posts shown on screen (刷到过). */
  seen: boolean
}

function loadSettings(): Settings {
  try {
    const v = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? '{}')
    return { history: v?.history !== false, seen: v?.seen !== false }
  } catch {
    return { history: true, seen: true }
  }
}

const settingsRef = ref<Settings>(loadSettings())
export const settings = readonly(settingsRef)

export function setSettings(patch: Partial<Settings>) {
  settingsRef.value = { ...settingsRef.value, ...patch }
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settingsRef.value))
  } catch {
    // Applies to this visit only.
  }
}

export const setHistoryEnabled = (on: boolean) => setSettings({ history: on })

// Another tab changed a list or the settings.
window.addEventListener('storage', (e) => {
  for (const name of LIST_NAMES) if (e.key === key(name)) lists[name].value = load(name)
  if (e.key === SETTINGS_KEY) settingsRef.value = loadSettings()
})

export function entries(name: ListName): Readonly<Ref<readonly Entry[]>> {
  return lists[name]
}

const idSet = (name: ListName) => computed(() => new Set(lists[name].value.map((e) => e.post.id)))
const idSets: Record<ListName, Readonly<Ref<Set<number>>>> = {
  likes: idSet('likes'),
  saves: idSet('saves'),
  history: idSet('history'),
  seen: idSet('seen'),
}

export function has(name: ListName, id: number): boolean {
  return idSets[name].value.has(id)
}

function put(name: ListName, post: PostCard) {
  const rest = lists[name].value.filter((e) => e.post.id !== post.id)
  lists[name].value = [{ post: snapshot(post), at: Date.now() }, ...rest].slice(0, LIMITS[name])
  store(name, lists[name].value)
}

export function remove(name: ListName, id: number) {
  lists[name].value = lists[name].value.filter((e) => e.post.id !== id)
  store(name, lists[name].value)
}

/** Adds or removes; returns whether the post is now in the list. */
export function toggle(name: ListName, post: PostCard): boolean {
  if (has(name, post.id)) {
    remove(name, post.id)
    return false
  }
  put(name, post)
  return true
}

export function clear(name: ListName) {
  lists[name].value = []
  store(name, [])
}

export function recordVisit(post: PostCard) {
  if (settingsRef.value.history) put('history', post)
}

// 刷到过: cards come on screen many at a time while scrolling, so they are
// gathered and written a couple of seconds later in one go.
const shown = new Map<number, { post: PostCard; from: string; at: number }>()
let flushTimer: ReturnType<typeof setTimeout> | undefined

/** A card stayed on screen (see Waterfall); `from` says where, e.g. "最新". */
export function recordShown(post: PostCard, from: string) {
  if (!settingsRef.value.seen) return
  shown.set(post.id, { post, from, at: Date.now() })
  flushTimer ??= setTimeout(flushShown, 2000)
}

function flushShown() {
  clearTimeout(flushTimer)
  flushTimer = undefined
  if (!shown.size) return
  const old = new Map(lists.seen.value.map((e) => [e.post.id, e]))
  const fresh: Entry[] = [...shown.values()].map(({ post, from, at }) => ({
    post: snapshot(post, true),
    at,
    from,
    count: (old.get(post.id)?.count ?? 0) + 1,
  }))
  shown.clear()
  const ids = new Set(fresh.map((e) => e.post.id))
  lists.seen.value = [...fresh.sort((a, b) => b.at - a.at), ...lists.seen.value.filter((e) => !ids.has(e.post.id))].slice(
    0,
    LIMITS.seen,
  )
  store('seen', lists.seen.value)
}

document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'hidden') flushShown()
})
window.addEventListener('pagehide', flushShown)

/** Shown lately but never opened, liked or saved: recommendations pass them over a little. */
export function passedOverIds(max: number): number[] {
  const out: number[] = []
  for (const e of lists.seen.value) {
    if (out.length >= max) break
    const id = e.post.id
    if (!has('history', id) && !has('likes', id) && !has('saves', id)) out.push(id)
  }
  return out
}

/** Newest first, for personalised recommendations. */
export function recentIds(name: ListName, max: number): number[] {
  return lists[name].value.slice(0, max).map((e) => e.post.id)
}

/** Lower-cased text a search can match: title, excerpt, blog, tags, category. */
function haystack(p: PostCard): string {
  const c = p.category
  return [p.title, p.excerpt, p.blog.name, p.author ?? '', ...p.tags, ...(p.auto_tags ?? []), c.parent?.label ?? '', c.label]
    .join('\n')
    .toLowerCase()
}

export function searchList(name: ListName, q: string): Entry[] {
  const needle = q.trim().toLowerCase()
  if (!needle) return [...lists[name].value]
  return lists[name].value.filter((e) => haystack(e.post).includes(needle))
}

/** Replaces or merges a list (restoring a backup). Merging keeps the newer of duplicates. */
export function restoreList(name: ListName, incoming: Entry[], mode: 'merge' | 'replace') {
  const byId = new Map<number, Entry>()
  for (const e of mode === 'merge' ? [...lists[name].value, ...incoming] : incoming) {
    const prev = byId.get(e.post.id)
    if (!prev || e.at > prev.at) byId.set(e.post.id, e)
  }
  lists[name].value = [...byId.values()].sort((a, b) => b.at - a.at).slice(0, LIMITS[name])
  store(name, lists[name].value)
}

export const LIST_LABELS: Record<ListName, string> = { saves: '收藏', likes: '喜欢', history: '历史', seen: '刷到过' }
