// Static mode (VITE_STATIC=1): the site is plain files, e.g. on GitHub Pages.
// Same functions as the live API, answered in the browser from the JSON that
// `feed-server export` writes to <base>/data/. See backend/src/export.rs.

import type { BlogQuery, PostQuery } from './api'
import { entries } from './library'
import type { BlogDetail, BlogSummary, Meta, Page, PostCard, PostDetail, TagsInfo, TagStat } from './types'

const DATA = `${import.meta.env.BASE_URL}data/`
/** Shards (newest first) that "推荐" mixes from when nothing narrows it down. */
const DISCOVER_SHARDS = 2
/**
 * As on the server (backend/src/api.rs): days of freshness a perfect match
 * with the reader's interests and a blog they know are worth (exploring turns
 * both round), how far back a post already seen goes, posts per blog.
 */
const LEANINGS = {
  discover: { interest: 40, blog: 5, perBlog: 8 },
  explore: { interest: -40, blog: -20, perBlog: 3 },
}
const SEEN_PENALTY_DAYS = 60
const SHOWN_PENALTY_DAYS = 10
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
const blogFile = (id: number) => file<BlogFile>(`blogs/${id}.json`)
const allBlogs = () => file<BlogSummary[]>('blogs.json')

async function allPosts(): Promise<PostCard[]> {
  const idx = await index()
  return (await Promise.all(Array.from({ length: idx.feed_shards }, (_, i) => shard(i)))).flat()
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

function postFilter(q: PostQuery) {
  const blogs = q.blogs ? new Set(q.blogs) : null
  const tag = q.tag?.trim().toLowerCase()
  const cat = q.category
  return (p: PostCard) =>
    (!q.dirtag || (p.blog.tags ?? []).includes(q.dirtag)) &&
    (!q.media || (q.media === 'cover') === !!p.cover) &&
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
 * and opened posts (kept in this browser, so they need not be in the feed).
 */
async function interests(q: PostQuery): Promise<Map<string, number>> {
  const terms = new Map<string, number>()
  const add = (term: string, w: number) => terms.set(term, (terms.get(term) ?? 0) + w)
  const addPost = (p: PostCard, w: number) => {
    add(`cat:${p.category.id}`, w)
    if (p.category.parent) add(`cat:${p.category.parent.id}`, w)
    for (const t of tagsOf(p)) add(`tag:${lower(t)}`, w)
  }
  if (q.follow?.length) {
    const followed = new Set(q.follow)
    for (const b of await allBlogs()) {
      if (followed.has(b.id)) for (const c of b.categories ?? []) add(`cat:${c.id}`, WEIGHTS.follow * (c.share ?? 0.5))
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
  return terms
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
  const pool = (await Promise.all(Array.from({ length: n }, (_, i) => shard(i)))).flat()
  const terms = await interests(q)
  const explore = q.mode === 'explore'
  const leaning = LEANINGS[explore ? 'explore' : 'discover']
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
  const perBlog = new Map<number, number>()
  const scored: { p: PostCard; score: number }[] = []
  for (const p of pool.filter(postFilter(q))) {
    const rn = (perBlog.get(p.blog.id) ?? 0) + 1
    perBlog.set(p.blog.id, rn)
    if (rn > leaning.perBlog) continue
    const score =
      p.sort_at / 86400 +
      noise(p.id, seed) * 14 -
      (rn - 1) * 5 +
      similarity(p, terms) * leaning.interest +
      (known.has(p.blog.id) ? leaning.blog : 0) -
      (seen.has(p.id) ? SEEN_PENALTY_DAYS : 0) -
      (shown.has(p.id) ? SHOWN_PENALTY_DAYS : 0)
    scored.push({ p, score: p.undated ? score - 1e9 : score })
  }
  scored.sort((a, b) => b.score - a.score || b.p.id - a.p.id)
  return page(
    scored.map((s) => s.p),
    q.cursor,
    limit,
  )
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
  if (q.blog !== undefined) return page((await blogFile(q.blog)).posts, q.cursor, limit)

  const search = q.q?.trim().toLowerCase()
  const newestFirst = q.mode === 'latest' || !!search || !!q.blogs
  if (!newestFirst) return discover(q, limit)

  const filter = postFilter(q)
  const cursor = q.cursor ?? (q.startAt ? String(await positionOf(q.startAt)) : null)
  return scan(
    (p) =>
      filter(p) &&
      (!search ||
        lower(p.title).includes(search) ||
        lower(p.excerpt).includes(search) ||
        lower(p.blog.name).includes(search) ||
        tagsOf(p).some((t) => lower(t) === search)),
    cursor,
    limit,
  )
}

async function post(id: number): Promise<PostDetail> {
  const idx = await index()
  for (let s = 0; s < idx.feed_shards; s++) {
    const found = (await shard(s)).find((p) => p.id === id)
    if (found) return { ...found, content_html: null }
  }
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
  related,
  blogs,
  blog: async (id: number) => (await blogFile(id)).blog,
}
