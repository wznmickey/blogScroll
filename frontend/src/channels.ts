// The home page's channel bar: which channels show, in what order, and which
// are pinned (kept at the left, outside the part that scrolls). Kept in this
// browser; included in backups.

import { ref, watch } from 'vue'
import type { TagsInfo } from './types'

/**
 * `foryou` (personalised), `explore` (the reverse), `discover`, `latest`, `following`,
 * `cat:tech`, `cat:tech/ai`, `kind:podcast`, `tag:Rust`.
 */
export type ChannelId = string

export interface Channel {
  id: ChannelId
  label: string
  /** Longer name for the manager, e.g. "技术 › AI". */
  title: string
  type: 'mode' | 'category' | 'kind' | 'tag'
}

const MODES: Channel[] = [
  { id: 'foryou', label: '推荐', title: '推荐 · 按你的喜好', type: 'mode' },
  { id: 'explore', label: '探索', title: '探索 · 你还没看过的方向', type: 'mode' },
  { id: 'discover', label: '发现', title: '发现 · 不做个性化', type: 'mode' },
  { id: 'latest', label: '最新', title: '最新', type: 'mode' },
  { id: 'following', label: '关注', title: '关注的博主', type: 'mode' },
]

export const isMode = (id: ChannelId) => MODES.some((m) => m.id === id)

/** Every channel the site offers (tag channels only once added). */
export function allChannels(info: TagsInfo | null): Channel[] {
  const out = [...MODES]
  for (const c of info?.categories ?? []) {
    out.push({ id: `cat:${c.id}`, label: c.label, title: c.label, type: 'category' })
  }
  for (const c of info?.categories ?? []) {
    for (const s of c.children ?? []) {
      out.push({ id: `cat:${s.id}`, label: s.label, title: `${c.label} › ${s.label}`, type: 'category' })
    }
  }
  for (const k of info?.kinds ?? []) {
    if (k.id !== 'blog') out.push({ id: `kind:${k.id}`, label: k.label, title: `类型：${k.label}`, type: 'kind' })
  }
  return out
}

/** A channel by id, also for ids not offered (yet), e.g. a tag from a link. */
export function channel(id: ChannelId, info: TagsInfo | null): Channel {
  const known = allChannels(info).find((c) => c.id === id)
  if (known) return known
  if (id.startsWith('tag:')) {
    const name = id.slice(4)
    return { id, label: `#${name}`, title: `标签 #${name}`, type: 'tag' }
  }
  if (id.startsWith('kind:')) return { id, label: id.slice(5), title: id.slice(5), type: 'kind' }
  return { id, label: id.replace(/^cat:/, ''), title: id, type: 'category' }
}

interface Prefs {
  /** Shown channels in order; null = the default list (follows new categories). */
  order: ChannelId[] | null
  pinned: ChannelId[]
  /** Posts with a cover image only, text-only posts only, or all. */
  media: Media
}

/** All posts, or only those with a cover image. */
export type Media = 'all' | 'cover'

const KEY = 'channels'
const DEFAULT_PINNED = ['foryou']
export const MAX_PINNED = 3

function defaultOrder(info: TagsInfo | null): ChannelId[] {
  return [
    ...MODES.map((m) => m.id),
    ...(info?.categories ?? []).filter((c) => c.id !== 'other').map((c) => `cat:${c.id}`),
  ]
}

export function cleanPrefs(v: unknown): Prefs {
  const o = (v ?? {}) as Partial<Prefs>
  const ids = (x: unknown) =>
    Array.isArray(x) ? [...new Set(x.filter((s): s is string => typeof s === 'string' && s.length < 200))] : null
  return {
    order: ids(o.order),
    pinned: (ids(o.pinned) ?? DEFAULT_PINNED).slice(0, MAX_PINNED),
    // Older settings had only "only with images" (`cover: true`).
    media: o.media === 'cover' || (o as { cover?: unknown }).cover === true ? 'cover' : 'all',
  }
}

function load(): Prefs {
  try {
    return cleanPrefs(JSON.parse(localStorage.getItem(KEY) ?? 'null'))
  } catch {
    return cleanPrefs(null)
  }
}

export const prefs = ref<Prefs>(load())

watch(
  prefs,
  (p) => {
    try {
      localStorage.setItem(KEY, JSON.stringify(p))
    } catch {
      // Applies to this visit only.
    }
  },
  { deep: true },
)

window.addEventListener('storage', (e) => {
  if (e.key === KEY) prefs.value = load()
})

/** The channels shown in the bar: pinned first, then the rest in order. */
export function shownChannels(info: TagsInfo | null): { pinned: Channel[]; rest: Channel[] } {
  const order = prefs.value.order ?? defaultOrder(info)
  const offered = new Set(allChannels(info).map((c) => c.id))
  // Categories and kinds disappear if the site stops having them; tags stay.
  const usable = order.filter((id) => offered.has(id) || id.startsWith('tag:') || !info)
  const pinned = prefs.value.pinned.filter((id) => usable.includes(id))
  return {
    pinned: pinned.map((id) => channel(id, info)),
    rest: usable.filter((id) => !pinned.includes(id)).map((id) => channel(id, info)),
  }
}

/** The first channel of the bar: what the home page shows by default. */
export function homeChannel(info: TagsInfo | null): ChannelId {
  const { pinned, rest } = shownChannels(info)
  return (pinned[0] ?? rest[0])?.id ?? 'foryou'
}

function currentOrder(info: TagsInfo | null): ChannelId[] {
  const { pinned, rest } = shownChannels(info)
  return [...pinned, ...rest].map((c) => c.id)
}

export function isShown(id: ChannelId, info: TagsInfo | null): boolean {
  return currentOrder(info).includes(id)
}

export function addChannel(id: ChannelId, info: TagsInfo | null) {
  const order = currentOrder(info)
  if (!order.includes(id)) prefs.value.order = [...order, id]
}

export function removeChannel(id: ChannelId, info: TagsInfo | null) {
  const order = currentOrder(info).filter((x) => x !== id)
  prefs.value.order = order.length ? order : ['foryou']
  prefs.value.pinned = prefs.value.pinned.filter((x) => x !== id)
}

/** Moves a channel one place within its group (pinned or not). */
export function moveChannel(id: ChannelId, delta: -1 | 1, info: TagsInfo | null) {
  const pinned = prefs.value.pinned.filter((x) => isShown(x, info))
  if (pinned.includes(id)) {
    const i = pinned.indexOf(id)
    const j = i + delta
    if (j < 0 || j >= pinned.length) return
    ;[pinned[i], pinned[j]] = [pinned[j], pinned[i]]
    prefs.value.pinned = pinned
    return
  }
  const rest = currentOrder(info).filter((x) => !pinned.includes(x))
  const i = rest.indexOf(id)
  const j = i + delta
  if (i < 0 || j < 0 || j >= rest.length) return
  ;[rest[i], rest[j]] = [rest[j], rest[i]]
  prefs.value.order = [...pinned, ...rest]
}

/** Returns false when the pin limit is reached. */
export function togglePin(id: ChannelId, info: TagsInfo | null): boolean {
  if (prefs.value.pinned.includes(id)) {
    prefs.value.pinned = prefs.value.pinned.filter((x) => x !== id)
    return true
  }
  if (prefs.value.pinned.filter((x) => isShown(x, info)).length >= MAX_PINNED) return false
  addChannel(id, info)
  prefs.value.pinned = [...prefs.value.pinned, id]
  return true
}

export function resetChannels() {
  prefs.value = { order: null, pinned: [...DEFAULT_PINNED], media: prefs.value.media }
}

/** Old home page links (?tab=, ?topic=, ?category=, ?tag=) as today's channel and filters. */
const OLD_TOPICS: Record<string, string> = {
  dev: 'tech/dev',
  ai: 'tech/ai',
  ops: 'tech/ops',
  security: 'tech/security',
  web: 'tech/web',
  gadget: 'gadget',
  study: 'study',
  life: 'life/daily',
  travel: 'travel',
  photo: 'photo',
  culture: 'culture/reading',
  acg: 'culture/acg',
  food: 'life/food',
  career: 'career',
  design: 'design',
  sports: 'life/sports',
}

export function upgradeOldQuery(q: Record<string, unknown>): Record<string, string> | null {
  const str = (v: unknown) => (typeof v === 'string' ? v : '')
  if (!('tab' in q || 'topic' in q || 'category' in q || 'tag' in q || 'kind' in q)) return null
  const out: Record<string, string> = {}
  if (str(q.p)) out.p = str(q.p)
  if (str(q.tag)) out.dirtag = str(q.tag)
  if (str(q.category)) out.c = `tag:${str(q.category)}`
  else if (str(q.topic)) out.c = `cat:${OLD_TOPICS[str(q.topic)] ?? str(q.topic)}`
  else if (str(q.kind)) out.c = `kind:${str(q.kind)}`
  else if (str(q.tab) === 'latest' || str(q.tab) === 'following') out.c = str(q.tab)
  return out
}
