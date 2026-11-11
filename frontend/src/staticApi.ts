// Static mode (VITE_STATIC=1): the site is plain files, e.g. on GitHub Pages.
// Same functions as the live API, answered in the browser from the JSON that
// `feed-server export` writes to <base>/data/. See backend/src/export.rs.

import type { BlogQuery, InterestQuery, PostQuery } from './api'
import { hasLongExcerpt } from './format'
import { DEFAULT_WEIGHTS, hostKey, type Weights } from './options'
import { entries } from './library'
import { noteCrawled, ownFeeds } from './sources'
import type {
  BlogDetail,
  BlogSummary,
  InterestProfile,
  Meta,
  Page,
  PostCard,
  PostDetail,
  TagsInfo,
  TagStat,
  Why,
} from './types'

const DATA = `${import.meta.env.BASE_URL}data/`
/** Shards (newest first) that "推荐" mixes from when nothing narrows it down. */
const DISCOVER_SHARDS = 2
/** How much each signal counts towards the reader's interests (as on the server). */
const WEIGHTS = { like: 3, save: 3, read: 1, follow: 2 }

interface Index {
  generated_at: number
  feed_posts: number
  feed_shards: number
  shard_size: number
}

interface BlogFile {
  blog: BlogDetail
  posts: PostCard[]
}

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return (await res.json()) as T
}

// index.json is always revalidated; every other file carries its export time
// as ?v=, so a new export is picked up but unchanged files stay cacheable.
let indexPromise: Promise<Index> | null = null
function index(): Promise<Index> {
  indexPromise ??= fetchJson<Index>(`${DATA}index.json`, { cache: 'no-cache' }).catch((e) => {
    indexPromise = null
    throw e
  })
  return indexPromise
}

const files = new Map<string, Promise<unknown>>()
async function file<T>(name: string): Promise<T> {
  const { generated_at } = await index()
  let p = files.get(name)
  if (!p) {
    p = fetchJson<T>(`${DATA}${name}?v=${generated_at}`)
    p.catch(() => files.delete(name))
    files.set(name, p)
  }
  return p as Promise<T>
}

const shard = (i: number) => file<PostCard[]>(`feed-${i}.json`)
const allBlogs = () => file<BlogSummary[]>('blogs.json')
/** A blog's page; the reader's own feeds that only they have are under own/. */
const blogFile = (id: number) => file<BlogFile>(`blogs/${id}.json`).catch(() => file<BlogFile>(`own/blogs/${id}.json`))

// The reader's own feeds, crawled like the site's blogs: each in a file named
// after the SHA-256 of its address (export.rs `own_file`), with its newest
// posts; none until the crawler has it.
interface OwnFile {
  feed: string
  blog: BlogDetail
  posts: PostCard[]
}
async function sha256(s: string): Promise<string> {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s))
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('')
}
const ownCache = new Map<string, Promise<OwnFile | null>>()
function ownFile(url: string): Promise<OwnFile | null> {
  let p = ownCache.get(url)
  if (!p) {
    p = sha256(url)
      .then((h) => file<OwnFile>(`own/${h}.json`))
      .then((f) => {
        noteCrawled(url, f.blog.id)
        return f
      })
      .catch(() => null)
    ownCache.set(url, p)
  }
  return p
}
async function ownFiles(): Promise<OwnFile[]> {
  if (!globalThis.crypto?.subtle) return []
  return (await Promise.all(ownFeeds.value.map((f) => ownFile(f.url)))).filter((f): f is OwnFile => !!f)
}
const newestFirst = (a: PostCard, b: PostCard) => b.sort_at - a.sort_at || b.id - a.id

/** The site's newest posts with the reader's own feeds' among them. */
async function withOwn(posts: PostCard[]): Promise<PostCard[]> {
  const own = (await ownFiles()).flatMap((f) => f.posts)
  if (!own.length) return posts
  const ids = new Set(posts.map((p) => p.id))
  return [...posts, ...own.filter((p) => !ids.has(p.id))].sort(newestFirst)
}

async function allPosts(): Promise<PostCard[]> {
  const idx = await index()
  return withOwn((await Promise.all(Array.from({ length: idx.feed_shards }, (_, i) => shard(i)))).flat())
}

function page<T>(all: T[], cursor: string | null | undefined, limit: number): Page<T> {
  const offset = Math.max(0, Number(cursor) || 0)
  return {
    items: all.slice(offset, offset + limit),
    next_cursor: offset + limit < all.length ? String(offset + limit) : null,
  }
}

const lower = (s: string) => s.toLowerCase()
const topOf = (category: string) => category.split('/')[0]
const tagsOf = (p: PostCard) => [...p.tags, ...(p.auto_tags ?? [])]

/** Which blogs each directory lists, for the reader's choice of sources. */
async function blogsIn(directories: string[]): Promise<Set<number>> {
  const want = new Set(directories)
  return new Set((await allBlogs()).filter((b) => (b.directory_ids ?? []).some((d) => want.has(d))).map((b) => b.id))
}

async function postFilter(q: PostQuery) {
  const blogs = q.blogs ? new Set(q.blogs) : null
  const tag = q.tag?.trim().toLowerCase()
  const cat = q.category
  const blocked = new Set(q.blockBlogs ?? [])
  const hosts = q.blockHosts ?? []
  const words = (q.blockWords ?? []).map(lower)
  const listed = q.directories ? await blogsIn(q.directories) : null
  // The reader's own feeds are not in a directory of the site's.
  for (const f of q.directories ? await ownFiles() : []) listed?.add(f.blog.id)
  const host = (url: string) => hostKey(url)
  return (p: PostCard) =>
    (q.nsfw || !p.nsfw) &&
    !blocked.has(p.blog.id) &&
    (!listed || listed.has(p.blog.id)) &&
    (!hosts.length || !hosts.some((h) => host(p.blog.homepage) === h || host(p.blog.homepage).endsWith(`.${h}`))) &&
    (!words.length ||
      !words.some(
        (w) => lower(p.title).includes(w) || lower(p.excerpt).includes(w) || tagsOf(p).some((t) => lower(t) === w),
      )) &&
    (!q.dirtag || (p.blog.tags ?? []).includes(q.dirtag)) &&
    (!q.hide?.noCover || !!p.cover) &&
    (!q.hide?.shortExcerpt || hasLongExcerpt(p)) &&
    (!q.hide?.noComments || (p.comment_count ?? 0) > 0) &&
    (!cat || p.category.id === cat || p.category.id.startsWith(`${cat}/`)) &&
    (!q.kind || (p.blog.kind ?? 'blog') === q.kind) &&
    (!blogs || blogs.has(p.blog.id)) &&
    (!tag || tagsOf(p).some((t) => lower(t) === tag))
}

/** Same scoring as the server's discover mode (backend/src/api.rs). */
function noise(id: number, seed: number) {
  return ((id * 2654435761 + seed) % 1000003) / 1000003
}

/**
 * The reader's interests as weighted features (`cat:tech`, `cat:tech/ai`,
 * `tag:rust`), from followed blogs' categories and the cards of liked, saved
 * and opened posts (kept in this browser, so they need not be in the feed);
 * and each category's share of those signals, also under its top level.
 */
async function interests(q: InterestQuery): Promise<{ terms: Map<string, number>; categories: Map<string, number> }> {
  const terms = new Map<string, number>()
  const categories = new Map<string, number>()
  let total = 0
  const add = (term: string, w: number) => terms.set(term, (terms.get(term) ?? 0) + w)
  const addCategory = (id: string, parent: string | undefined, w: number) => {
    if (id === 'other') return
    categories.set(id, (categories.get(id) ?? 0) + w)
    if (parent) categories.set(parent, (categories.get(parent) ?? 0) + w)
    total += w
  }
  const addPost = (p: PostCard, w: number) => {
    add(`cat:${p.category.id}`, w)
    if (p.category.parent) add(`cat:${p.category.parent.id}`, w)
    for (const t of tagsOf(p)) add(`tag:${lower(t)}`, w)
    addCategory(p.category.id, p.category.parent?.id, w)
  }
  if (q.follow?.length) {
    const followed = new Set(q.follow)
    for (const b of await allBlogs()) {
      if (!followed.has(b.id)) continue
      for (const c of b.categories ?? []) {
        add(`cat:${c.id}`, WEIGHTS.follow * (c.share ?? 0.5))
        addCategory(c.id, undefined, WEIGHTS.follow * (c.share ?? 0.5))
      }
    }
  }
  for (const name of ['like', 'save', 'read'] as const) {
    const ids = new Set(q[name] ?? [])
    if (!ids.size) continue
    const list = entries(name === 'like' ? 'likes' : name === 'save' ? 'saves' : 'history').value
    for (const e of list) if (ids.has(e.post.id)) addPost(e.post, WEIGHTS[name])
  }
  const norm = Math.sqrt([...terms.values()].reduce((s, w) => s + w * w, 0))
  if (norm > 0) for (const [k, w] of terms) terms.set(k, w / norm)
  if (total > 0) for (const [k, w] of categories) categories.set(k, w / total)
  return { terms, categories }
}

/** The same picture of the reader as GET /api/interest, top terms first. */
async function interest(q: InterestQuery): Promise<InterestProfile> {
  const { terms, categories } = await interests(q)
  const top = [...terms].sort((a, b) => b[1] - a[1]).slice(0, 40)
  return {
    terms: Object.fromEntries(top.map(([k, w]) => [k.replace(/^(cat|tag):/, ''), Math.round(w * 10000) / 10000])),
    categories: Object.fromEntries([...categories].map(([k, w]) => [k, Math.round(w * 1000) / 1000])),
  }
}

function similarity(p: PostCard, terms: Map<string, number>) {
  if (!terms.size) return 0
  const keys = [`cat:${p.category.id}`, ...tagsOf(p).map((t) => `tag:${lower(t)}`)]
  if (p.category.parent) keys.push(`cat:${p.category.parent.id}`)
  const hit = keys.reduce((s, k) => s + (terms.get(k) ?? 0), 0)
  return hit / Math.sqrt(keys.length)
}

async function discover(q: PostQuery, limit: number): Promise<Page<PostCard>> {
  const idx = await index()
  // A narrow channel needs the whole feed to fill a page.
  const narrow = !!(q.tag || q.category || q.kind || q.dirtag)
  const n = narrow ? idx.feed_shards : Math.min(DISCOVER_SHARDS, idx.feed_shards)
  const pool = await withOwn((await Promise.all(Array.from({ length: n }, (_, i) => shard(i)))).flat())
  const { terms, categories } = await interests(q)
  const explore = q.mode === 'explore'
  const w = leaning(q.weights ?? DEFAULT_WEIGHTS, explore)
  const seen = new Set([...(q.like ?? []), ...(q.save ?? []), ...(q.read ?? [])])
  const shown = new Set(q.shown ?? [])
  // Blogs the reader knows: followed ones, and when exploring those they read.
  const known = new Set(q.follow ?? [])
  if (explore) {
    for (const name of ['likes', 'saves', 'history'] as const) {
      for (const e of entries(name).value) if (seen.has(e.post.id)) known.add(e.post.blog.id)
    }
  }
  const seed = Math.abs(q.seed ?? 0) % 2 ** 31
  const now = Date.now() / 1000
  const perBlog = new Map<number, number>()
  const scored: { p: PostCard; score: number }[] = []
  const filter = await postFilter(q)
  for (const p of pool.filter(filter)) {
    const rn = (perBlog.get(p.blog.id) ?? 0) + 1
    perBlog.set(p.blog.id, rn)
    if (rn > w.perBlog) continue
    const share = (categories.get(p.category.id) ?? 0) + (p.category.parent ? (categories.get(p.category.parent.id) ?? 0) : 0)
    const why: Why = {
      age_days: Math.round(((now - p.sort_at) / 86400) * 10) / 10,
      noise: noise(p.id, seed) * w.noise,
      same_blog: -(rn - 1) * w.sameBlog,
      interest: similarity(p, terms) * w.interest,
      category: share * w.category,
      blog: known.has(p.blog.id) ? w.blog : 0,
      seen: seen.has(p.id) ? -w.seen : 0,
      shown: shown.has(p.id) ? -w.shown : 0,
    }
    const score =
      p.sort_at / 86400 + why.noise + why.same_blog + why.interest + why.category + why.blog + why.seen + why.shown
    const round = (x: number) => Math.round(x * 10) / 10
    scored.push({
      p: { ...p, why: { ...why, noise: round(why.noise), interest: round(why.interest), category: round(why.category) } },
      score: p.undated ? score - 1e9 : score,
    })
  }
  scored.sort((a, b) => b.score - a.score || b.p.id - a.p.id)
  return page(
    scored.map((s) => s.p),
    q.cursor,
    limit,
  )
}

/** 推荐 as set; 反推荐 the other way round (as `Weights::leaning` on the server). */
function leaning(w: Weights, explore: boolean): Weights {
  if (!explore) return w
  return { ...w, interest: -w.interest, category: -2 * w.category, blog: -4 * w.blog, perBlog: Math.min(w.perBlog, 3) }
}

/** Position of the first post at or after `start` in the newest-first feed. */
async function positionOf(start: { sort_at: number; id: number }): Promise<number> {
  const idx = await index()
  for (let s = 0; s < idx.feed_shards; s++) {
    const posts = await shard(s)
    const i = posts.findIndex((p) => p.sort_at < start.sort_at || (p.sort_at === start.sort_at && p.id <= start.id))
    if (i >= 0) return s * idx.shard_size + i
  }
  return idx.feed_posts
}

/** Walks the shards newest-first from `cursor` (a global position), loading them as needed. */
async function scan(match: (p: PostCard) => boolean, cursor: string | null | undefined, limit: number) {
  const idx = await index()
  let pos = Math.max(0, Number(cursor) || 0)
  const items: PostCard[] = []
  while (pos < idx.feed_posts && items.length < limit) {
    const s = Math.floor(pos / idx.shard_size)
    const posts = await shard(s)
    let i = pos - s * idx.shard_size
    for (; i < posts.length && items.length < limit; i++) {
      if (match(posts[i])) items.push(posts[i])
    }
    pos = i >= posts.length ? (s + 1) * idx.shard_size : s * idx.shard_size + i
  }
  const next = Math.min(pos, idx.feed_posts)
  return { items, next_cursor: next < idx.feed_posts ? String(next) : null }
}

async function posts(q: PostQuery): Promise<Page<PostCard>> {
  const limit = q.limit ?? 30
  if (q.blog !== undefined) {
    return page(
      (await blogFile(q.blog)).posts.filter((p) => q.nsfw || !p.nsfw),
      q.cursor,
      limit,
    )
  }

  const search = q.q?.trim().toLowerCase()
  const newestFirst = q.mode === 'latest' || !!search || !!q.blogs
  if (!newestFirst) return discover(q, limit)

  const filter = await postFilter(q)
  const matches = (p: PostCard) =>
    filter(p) &&
    (!search ||
      lower(p.title).includes(search) ||
      lower(p.excerpt).includes(search) ||
      lower(p.blog.name).includes(search) ||
      tagsOf(p).some((t) => lower(t) === search))
  // With the reader's own feeds: all of it at once, theirs among the site's.
  if ((await ownFiles()).length) {
    const all = (await allPosts()).filter(matches)
    const start = q.startAt
    const at = start ? all.findIndex((p) => p.sort_at < start.sort_at || (p.sort_at === start.sort_at && p.id <= start.id)) : -1
    return page(all, q.cursor ?? (start ? String(at < 0 ? all.length : at) : null), limit)
  }
  const cursor = q.cursor ?? (q.startAt ? String(await positionOf(q.startAt)) : null)
  return scan(matches, cursor, limit)
}

async function post(id: number): Promise<PostDetail> {
  // Posts of blogs whose authors allowed full text or their comment widget
  // have their own file with all of it.
  const detailed = await file<number[]>('details.json').catch(() => [] as number[])
  if (detailed.includes(id)) return file<PostDetail>(`posts/${id}.json`)
  const found = (await allPosts()).find((p) => p.id === id)
  if (found) return { ...found, content_html: null }
  throw new Error('HTTP 404')
}

async function related(id: number, limit = 8): Promise<PostCard[]> {
  const map = await file<Record<string, number[]>>('related.json')
  const ids = (map[String(id)] ?? []).slice(0, limit)
  if (!ids.length) return []
  const byId = new Map((await allPosts()).map((p) => [p.id, p]))
  return ids.map((i) => byId.get(i)).filter((p): p is PostCard => !!p)
}

/** Lower is better, like the server's ranking for `q`. */
function matchRank(b: BlogSummary, needle: string): number | null {
  const name = lower(b.name)
  if (name === needle) return 0
  if (name.startsWith(needle)) return 1
  if (name.includes(needle)) return 2
  if (lower(b.homepage).includes(needle)) return 3
  if (b.description && lower(b.description).includes(needle)) return 4
  if ((b.categories ?? []).some((c) => c.label.includes(needle) || needle.includes(c.label))) return 5
  return null
}

async function searchTags(q: string, limit = 20): Promise<TagStat[]> {
  const needle = q.trim().toLowerCase()
  const all = await file<TagStat[]>('tag-index.json')
  const hits = needle ? all.filter((t) => lower(t.name).includes(needle)) : all
  // Exact match first; the index is already ordered by use.
  return [...hits.filter((t) => lower(t.name) === needle), ...hits.filter((t) => lower(t.name) !== needle)].slice(0, limit)
}

async function blogs(q: BlogQuery): Promise<Page<BlogSummary>> {
  const needle = q.q?.trim().toLowerCase()
  const ids = q.ids ? new Set(q.ids) : null
  const recent = (a: BlogSummary, b: BlogSummary) =>
    (b.last_post_at ?? -1) - (a.last_post_at ?? -1) || a.id - b.id
  const sorters = {
    recent,
    posts: (a: BlogSummary, b: BlogSummary) => b.post_count - a.post_count || recent(a, b),
    name: (a: BlogSummary, b: BlogSummary) => a.name.localeCompare(b.name, 'zh-CN'),
  }
  const ranks = new Map<number, number>()
  const all = (await allBlogs())
    .filter((b) => !q.dirtag || b.tags.some((t) => t.tag === q.dirtag))
    .filter((b) => !q.category || (b.categories ?? []).some((c) => c.id === topOf(q.category!)))
    .filter((b) => !q.kind || b.kind.id === q.kind)
    .filter((b) => !ids || ids.has(b.id))
    .filter((b) => {
      if (!needle) return true
      const r = matchRank(b, needle)
      if (r !== null) ranks.set(b.id, r)
      return r !== null
    })
    .sort((a, b) => (ranks.get(a.id) ?? 0) - (ranks.get(b.id) ?? 0) || sorters[q.sort ?? 'recent'](a, b))
  return page(all, String(q.offset ?? 0), q.limit ?? 48)
}

export const staticApi = {
  meta: () => file<Meta>('meta.json'),
  tags: () => file<TagsInfo>('tags.json'),
  searchTags,
  posts,
  post,
  interest,
  related,
  blogs,
  blog: async (id: number) => (await blogFile(id)).blog,
  blogByKey: async (key: string) => {
    const found = (await allBlogs()).find((b) => b.key === key) ?? (await ownFiles()).find((f) => f.blog.key === key)?.blog
    if (!found) throw new Error('HTTP 404')
    return (await blogFile(found.id)).blog
  },
}
