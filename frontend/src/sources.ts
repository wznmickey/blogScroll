// The reader's own sources (设置 → 来源 → 我添加的): feeds, and lists of
// feeds (OPML, from an address or a file), that only they see.
//
// Every feed is registered with the site's server (registerOwn), which then
// crawls it like the site's own blogs: whole history, categories, tags, 推荐.
// It stays off every list but theirs: their requests name it (`own=`, live
// site) or their browser reads its file (data/own/, static site). Until the
// crawler has it (`crawled`), and where the server does not crawl, their
// browser reads it itself through /api/fetch (server.ts), at most every half
// hour (a list once a day), for the 订阅 channel and a page per feed
// (/blog/-123: negative ids are those). The sources are synced and backed
// up; what was read from them is not.

import { computed, ref, watch } from 'vue'
import { report } from './changes'
import { hasLongExcerpt, hostOf } from './format'
import { parseOpmlFile, parseSource, type ListedFeed, type ParsedFeed, type ParsedItem } from './feedParse'
import { fetchRemote, FetchFailed, loadCapabilities } from './server'
import type { BlogDetail, BlogRef, Page, PostCard, PostDetail } from './types'
import type { Hide } from './channels'

export interface OwnSource {
  /** The feed's or the list's address; a list from a file: `file:<its name>`. */
  url: string
  kind: 'feed' | 'list'
  title: string
  homepage: string
  /** Added or last changed, ms. */
  at: number
  /** Lists: their feeds, as last read. */
  feeds?: ListedFeed[]
}

/** How reading a feed (or list) last went; not synced. */
interface Read {
  /** Last tried, and last read, ms. */
  tried: number
  ok: number
  error?: string
  /** Not again before this, ms (the site said 429 / 503). */
  wait?: number
  items: PostCard[]
}

const KEY = 'sources'
const READ_KEY = 'sources:read'
const CRAWLED_KEY = 'sources:crawled'
const REGISTERED_KEY = 'sources:registered'
const MAX_SOURCES = 300
const MAX_FEEDS = 500
const FEED_EVERY = 30 * 60_000
const LIST_EVERY = 24 * 3600_000
const FAILED_EVERY = 60 * 60_000
const KEEP_PER_FEED = 20
const OTHER = { id: 'other', label: '其他' }

const isAddress = (u: unknown): u is string => typeof u === 'string' && /^(https?:\/\/|file:)/.test(u) && u.length < 2000
const str = (v: unknown, max = 200) => (typeof v === 'string' ? v.slice(0, max) : '')

function cleanFeeds(v: unknown): ListedFeed[] {
  if (!Array.isArray(v)) return []
  return v
    .filter((f) => isAddress(f?.url) && !f.url.startsWith('file:'))
    .map((f) => ({ url: f.url, title: str(f.title), homepage: str(f.homepage, 2000) }))
    .slice(0, MAX_FEEDS)
}

/** Accepts sources from storage, a backup or another device, dropping anything malformed. */
export function cleanSources(v: unknown): OwnSource[] {
  if (!Array.isArray(v)) return []
  const seen = new Set<string>()
  const out: OwnSource[] = []
  for (const s of v) {
    if (!isAddress(s?.url) || seen.has(s.url) || (s.kind !== 'feed' && s.kind !== 'list')) continue
    seen.add(s.url)
    out.push({
      url: s.url,
      kind: s.kind,
      title: str(s.title) || hostOf(s.url),
      homepage: str(s.homepage, 2000),
      at: Number.isFinite(s.at) ? s.at : 0,
      ...(s.kind === 'list' ? { feeds: cleanFeeds(s.feeds) } : {}),
    })
  }
  return out.sort((a, b) => b.at - a.at).slice(0, MAX_SOURCES)
}

function load<T>(key: string, fallback: T): T {
  try {
    return JSON.parse(localStorage.getItem(key) ?? 'null') ?? fallback
  } catch {
    return fallback
  }
}
function store(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // Storage full or blocked: kept for this visit.
  }
}

export const ownSources = ref<OwnSource[]>(cleanSources(load(KEY, [])))
const reads = ref<Record<string, Read>>(load(READ_KEY, {}))

window.addEventListener('storage', (e) => {
  if (e.key === KEY) ownSources.value = cleanSources(load(KEY, []))
  if (e.key === READ_KEY) reads.value = load(READ_KEY, {})
})

/** Putting sources in place from a sync or a backup (not a change of the reader's). */
export function setOwnSources(list: OwnSource[]) {
  ownSources.value = cleanSources(list)
  store(KEY, ownSources.value)
}

function changed(list: OwnSource[]) {
  ownSources.value = cleanSources(list)
  store(KEY, ownSources.value)
  report({ kind: 'changed' })
}

/** Every feed to read: those added on their own, then those of the lists. */
export const ownFeeds = computed(() => {
  const out = new Map<string, ListedFeed & { list?: string }>()
  for (const s of ownSources.value) if (s.kind === 'feed') out.set(s.url, { url: s.url, title: s.title, homepage: s.homepage })
  for (const s of ownSources.value) {
    for (const f of s.feeds ?? []) if (!out.has(f.url)) out.set(f.url, { ...f, list: s.title })
  }
  return [...out.values()].slice(0, MAX_FEEDS)
})

/** Which of the reader's feeds the site crawls itself, as which blog. */
export const crawled = ref<Record<string, number>>(load(CRAWLED_KEY, {}))

export function noteCrawled(url: string, id: number) {
  if (crawled.value[url] === id) return
  crawled.value = { ...crawled.value, [url]: id }
  store(CRAWLED_KEY, crawled.value)
}

/** The site's blogs that are the reader's own feeds: asked for by id (`own=`). */
export function crawledIds(): number[] {
  const mine = new Set(ownFeeds.value.map((f) => f.url))
  return [...new Set(Object.entries(crawled.value).filter(([u]) => mine.has(u)).map(([, id]) => id))]
}

/** The feed a blog of the site is, when it is one of the reader's (for asking to read it again). */
export const crawledFeed = (blogId: number) => Object.entries(crawled.value).find(([, id]) => id === blogId)?.[0]

/** Feeds the site does not crawl (yet): read here. */
const previewFeeds = computed(() => ownFeeds.value.filter((f) => !crawled.value[f.url]))

/**
 * Tells the server every feed the reader has, so the crawler takes in the
 * new ones and keeps the rest (it drops what nobody asked for in months);
 * learns which blog each crawled one is. Without who the reader is.
 */
export async function registerOwn(): Promise<void> {
  const feeds = ownFeeds.value.map((f) => f.url)
  if (!feeds.length || !(await loadCapabilities()).readers) return
  try {
    const res = await fetch('/api/readers/feeds', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ feeds }),
    })
    if (!res.ok) return
    const answer = await res.json()
    for (const [url, id] of Object.entries(answer?.blogs ?? {})) {
      if (Number.isInteger(id) && (id as number) > 0) noteCrawled(url, id as number)
    }
    store(REGISTERED_KEY, Date.now())
  } catch {
    // Next time.
  }
}

let registerTimer: ReturnType<typeof setTimeout> | undefined
watch(
  () => ownFeeds.value.map((f) => f.url).join('\n'),
  () => {
    clearTimeout(registerTimer)
    registerTimer = setTimeout(registerOwn, 2000)
  },
)
// Once a day anyway, to say they are still wanted.
if (Date.now() - load<number>(REGISTERED_KEY, 0) > 86_400_000) setTimeout(registerOwn, 5000)

function hash(s: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return h >>> 0
}
export const ownBlogId = (feedUrl: string) => -(hash(feedUrl) % 2_000_000_000) - 1
const ownPostId = (feedUrl: string, guid: string) => -(hash(`${feedUrl}\n${guid}`) % 2_000_000_000) - 1

function cards(feed: ListedFeed, parsed: ParsedFeed, before: PostCard[], blog?: BlogRef): PostCard[] {
  const old = new Map(before.map((p) => [p.id, p]))
  const now = Math.floor(Date.now() / 1000)
  return parsed.items.slice(0, KEEP_PER_FEED).map((it: ParsedItem, i) => {
    const id = ownPostId(feed.url, it.guid)
    return {
      id,
      title: it.title,
      url: it.url || feed.homepage,
      excerpt: it.excerpt,
      cover: it.cover,
      author: it.author,
      tags: it.tags,
      auto_tags: [],
      category: OTHER,
      published_at: it.published,
      // Undated: when first read, in the feed's order.
      sort_at: it.published !== null ? Math.min(it.published, now) : (old.get(id)?.sort_at ?? now - i),
      undated: it.published === null,
      has_full_text: false,
      comment_count: it.comments,
      comments_url: it.commentsUrl,
      blog: blog ?? {
        id: ownBlogId(feed.url),
        name: feed.title || parsed.title || hostOf(feed.homepage || feed.url),
        homepage: feed.homepage || parsed.homepage,
        icon: null,
        kind: 'blog',
        framable: null,
      },
    }
  })
}

/**
 * A blog of the site's feed as it is right now, read here (after asking for
 * it to be read again, before the site has): its posts as cards of that
 * blog, with ids of their own (negative) until the site has them.
 */
export async function readNow(feedUrl: string, blog: BlogRef): Promise<PostCard[]> {
  const got = await fetchRemote(feedUrl)
  if (got.kind !== 'feed') return []
  const parsed = parseSource(got.text, got.url)
  return 'feed' in parsed ? cards({ url: feedUrl, title: blog.name, homepage: blog.homepage }, parsed.feed, [], blog) : []
}

function noteRead(url: string, r: Read) {
  reads.value = { ...reads.value, [url]: r }
  store(READ_KEY, reads.value)
}

function failed(url: string, e: unknown) {
  const prev = reads.value[url]
  const wait = e instanceof FetchFailed && e.retryAfter ? Date.now() + e.retryAfter * 1000 : undefined
  noteRead(url, { tried: Date.now(), ok: prev?.ok ?? 0, items: prev?.items ?? [], error: (e as Error).message, wait })
}

async function readFeed(feed: ListedFeed) {
  try {
    const got = await fetchRemote(feed.url)
    if (got.kind !== 'feed') throw new Error('这个地址现在是网页，不是 RSS')
    const parsed = parseSource(got.text, got.url)
    if (!('feed' in parsed)) throw new Error('这是订阅列表，不是 RSS')
    noteRead(feed.url, { tried: Date.now(), ok: Date.now(), items: cards(feed, parsed.feed, reads.value[feed.url]?.items ?? []) })
  } catch (e) {
    failed(feed.url, e)
  }
}

async function readList(list: OwnSource) {
  try {
    const got = await fetchRemote(list.url)
    if (got.kind !== 'feed') throw new Error('这个地址现在是网页，不是 OPML')
    const parsed = parseSource(got.text, got.url)
    if (!('list' in parsed)) throw new Error('这个地址现在是 RSS，不是订阅列表')
    noteRead(list.url, { tried: Date.now(), ok: Date.now(), items: [] })
    const feeds = parsed.list.feeds.slice(0, MAX_FEEDS)
    if (JSON.stringify(feeds) !== JSON.stringify(list.feeds)) {
      changed(ownSources.value.map((s) => (s.url === list.url ? { ...s, feeds, at: Date.now() } : s)))
    }
  } catch (e) {
    failed(list.url, e)
  }
}

async function inTurn<T>(items: T[], run: (t: T) => Promise<void>, width: number) {
  const queue = [...items]
  await Promise.all(Array.from({ length: Math.min(width, queue.length) }, async () => {
    for (let t = queue.shift(); t !== undefined; t = queue.shift()) await run(t)
  }))
}

export const reading = ref(false)
let pass: Promise<void> | null = null

/** Reads what is due (or, with `force`, everything the sites allow now). */
export function refreshOwn(force = false): Promise<void> {
  pass ??= (async () => {
    reading.value = true
    try {
      const now = Date.now()
      const due = (url: string, every: number) => {
        const r = reads.value[url]
        return !r || ((r.wait ?? 0) <= now && (force || now - r.tried >= (r.error ? FAILED_EVERY : every)))
      }
      const lists = ownSources.value.filter((s) => s.kind === 'list' && !s.url.startsWith('file:') && due(s.url, LIST_EVERY))
      await inTurn(lists, readList, 2)
      await inTurn(previewFeeds.value.filter((f) => due(f.url, FEED_EVERY)), readFeed, 4)
    } finally {
      reading.value = false
      pass = null
    }
    // What was just read can be registered now (the Worker takes only feeds
    // it has fetched), and the crawler may have some by now.
    void registerOwn()
  })()
  return pass
}

export function readState(url: string): { ok: number; error?: string; count: number } | null {
  const r = reads.value[url]
  return r ? { ok: r.ok, error: r.error, count: r.items.length } : null
}

function allItems(): PostCard[] {
  return previewFeeds.value
    .flatMap((f) => reads.value[f.url]?.items ?? [])
    .sort((a, b) => b.sort_at - a.sort_at || a.id - b.id)
}

export interface OwnQuery {
  cursor?: string | null
  limit?: number
  /** One feed (its blog id). */
  blog?: number
  hide?: Hide
  blockWords?: string[]
}

/** What the reader's filters (channel bar, blocked words) leave of `list`. */
export function keepOwn(list: PostCard[], q: OwnQuery): PostCard[] {
  const words = (q.blockWords ?? []).map((w) => w.toLowerCase())
  return list.filter(
    (p) =>
      (q.blog === undefined || p.blog.id === q.blog) &&
      (!q.hide?.noCover || !!p.cover) &&
      (!q.hide?.shortExcerpt || hasLongExcerpt(p)) &&
      (!q.hide?.noComments || (p.comment_count ?? 0) > 0) &&
      !words.some((w) => `${p.title}\n${p.excerpt}`.toLowerCase().includes(w) || p.tags.some((t) => t.toLowerCase() === w)),
  )
}

export function pageOf(all: PostCard[], cursor: string | null | undefined, limit = 30): Page<PostCard> {
  const offset = Math.max(0, Number(cursor) || 0)
  return { items: all.slice(offset, offset + limit), next_cursor: offset + limit < all.length ? String(offset + limit) : null }
}

/** Posts read here from the feeds the site does not crawl (yet), newest first. */
export const previewPosts = (q: OwnQuery) => keepOwn(allItems(), q)

export function ownPosts(q: OwnQuery): Page<PostCard> {
  return pageOf(previewPosts(q), q.cursor, q.limit)
}

export function ownPost(id: number): PostCard | undefined {
  for (const f of ownFeeds.value) {
    const p = reads.value[f.url]?.items.find((x) => x.id === id)
    if (p) return p
  }
  return undefined
}

export const ownDetail = (id: number): PostDetail | undefined => {
  const p = ownPost(id)
  return p && { ...p, content_html: null }
}

/** A feed's page (BlogView), as the site's blogs have. */
export function ownBlog(id: number): BlogDetail | undefined {
  const f = ownFeeds.value.find((x) => ownBlogId(x.url) === id)
  if (!f) return undefined
  const r = reads.value[f.url]
  return {
    id,
    name: f.title || hostOf(f.homepage || f.url),
    homepage: f.homepage || r?.items[0]?.blog.homepage || f.url,
    icon: null,
    post_count: r?.items.length ?? 0,
    last_post_at: r?.items[0]?.sort_at ?? null,
    has_feed: true,
    description: null,
    tags: [],
    kind: { id: 'blog', label: '博客' },
    categories: [],
    framable: null,
    full_text: false,
    directories: [],
    about_url: null,
    about_text: null,
    social: [],
    generator: null,
    comment_system: null,
    top_tags: [],
    own: { feed: f.url, list: f.list ?? null, error: r?.error ?? null, read_at: r?.ok || null },
  }
}

/** An address as typed: a feed, a list, or a page that links to its feed. */
export interface Found {
  source: OwnSource
  /** A feed: what it holds now. */
  feed?: ParsedFeed
}

export async function lookUp(input: string): Promise<Found> {
  let url = input.trim()
  if (!/^https?:\/\//i.test(url)) url = `https://${url}`
  let got = await fetchRemote(url)
  if (got.kind === 'page') {
    if (!got.feeds.length) throw new Error('这个网页上没有找到 RSS')
    url = got.feeds[0]
    got = await fetchRemote(url)
    if (got.kind !== 'feed') throw new Error('这个网页上没有找到 RSS')
  }
  // Known by the address asked for (redirects or not): the crawler finds a
  // feed it already has by it, with its history.
  got = { ...got, url: new URL(url).href }
  const parsed = parseSource(got.text, got.url)
  const at = Date.now()
  if ('list' in parsed) {
    if (!parsed.list.feeds.length) throw new Error('这个订阅列表是空的')
    return { source: { url: got.url, kind: 'list', title: parsed.list.title || hostOf(got.url), homepage: '', at, feeds: parsed.list.feeds } }
  }
  const f = parsed.feed
  return { source: { url: got.url, kind: 'feed', title: f.title || hostOf(f.homepage), homepage: f.homepage, at }, feed: f }
}

/** A list from an OPML file the reader picked. */
export async function fromFile(file: File): Promise<Found> {
  const list = parseOpmlFile(await file.text())
  if (!list.feeds.length) throw new Error('这个文件里没有 RSS 地址')
  return { source: { url: `file:${file.name}`, kind: 'list', title: list.title || file.name, homepage: '', at: Date.now(), feeds: list.feeds } }
}

export function addOwn(found: Found) {
  const s = { ...found.source, at: Date.now() }
  changed([s, ...ownSources.value.filter((x) => x.url !== s.url)])
  if (found.feed) noteRead(s.url, { tried: Date.now(), ok: Date.now(), items: cards(s, found.feed, []) })
  void refreshOwn()
  // A server that crawls reads it at once: which blog it is, a little later.
  setTimeout(registerOwn, 30_000)
}

export function removeOwn(url: string) {
  changed(ownSources.value.filter((s) => s.url !== url))
  report({ kind: 'removed', collection: 'sources', keys: [url] })
}

export const hasOwn = () => ownSources.value.length > 0

/** As the site names its blogs (util.rs `site_key`): example.com/blog. */
export function siteKey(url: string): string | null {
  try {
    const u = new URL(url)
    const path = u.pathname.replace(/\/+$/, '').replace(/\/index\.html?$/, '')
    return `${u.hostname.toLowerCase().replace(/^www\./, '')}${path}`
  } catch {
    return null
  }
}
