// Static mode (VITE_STATIC=1): the site is plain files, e.g. on GitHub Pages.
// Same functions as the live API, answered in the browser from the JSON that
// `feed-server export` writes to <base>/data/. See backend/src/export.rs.

import type { BlogQuery, PostQuery } from './api'
import type { BlogDetail, BlogSummary, Meta, Page, PostCard, PostDetail, TagsInfo } from './types'

const DATA = `${import.meta.env.BASE_URL}data/`
/** Shards (newest first) that "推荐" mixes from. */
const DISCOVER_SHARDS = 2
/** Days of freshness a perfect match with the reader's interests is worth (as on the server). */
const INTEREST_BOOST_DAYS = 40

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

function postFilter(q: PostQuery) {
  const blogs = q.blogs ? new Set(q.blogs) : null
  const category = q.category?.trim().toLowerCase()
  return (p: PostCard) =>
    (!q.tag || (p.blog.tags ?? []).includes(q.tag)) &&
    (!q.cover || !!p.cover) &&
    (!q.topic || (p.topics ?? []).some((t) => t.id === q.topic)) &&
    (!q.kind || (p.blog.kind ?? 'blog') === q.kind) &&
    (!blogs || blogs.has(p.blog.id)) &&
    (!category || [...p.tags, ...(p.auto_tags ?? [])].some((t) => lower(t) === category))
}

/** Same scoring as the server's discover mode (backend/src/api.rs). */
function noise(id: number, seed: number) {
  return ((id * 2654435761 + seed) % 1000003) / 1000003
}

/** Reader interests as weighted terms: followed blogs' areas, opened posts' tags and areas. */
async function interests(q: PostQuery, pool: PostCard[]): Promise<Map<string, number>> {
  const terms = new Map<string, number>()
  const add = (term: string, w: number) => terms.set(term, (terms.get(term) ?? 0) + w)
  if (q.follow?.length) {
    const followed = new Set(q.follow)
    for (const b of await allBlogs()) {
      if (followed.has(b.id)) for (const t of b.topics) add(`topic:${t.id}`, t.share ?? 0.5)
    }
  }
  if (q.read?.length) {
    const read = new Set(q.read)
    for (const p of pool) {
      if (!read.has(p.id)) continue
      for (const t of p.topics ?? []) add(`topic:${t.id}`, 1)
      for (const t of [...p.tags, ...(p.auto_tags ?? [])]) add(`tag:${lower(t)}`, 1)
    }
  }
  const norm = Math.sqrt([...terms.values()].reduce((s, w) => s + w * w, 0))
  if (norm > 0) for (const [k, w] of terms) terms.set(k, w / norm)
  return terms
}

function similarity(p: PostCard, terms: Map<string, number>) {
  if (!terms.size) return 0
  const keys = [
    ...(p.topics ?? []).map((t) => `topic:${t.id}`),
    ...[...p.tags, ...(p.auto_tags ?? [])].map((t) => `tag:${lower(t)}`),
  ]
  if (!keys.length) return 0
  const hit = keys.reduce((s, k) => s + (terms.get(k) ?? 0), 0)
  return hit / Math.sqrt(keys.length)
}

async function discover(q: PostQuery, limit: number): Promise<Page<PostCard>> {
  const idx = await index()
  const n = Math.min(DISCOVER_SHARDS, idx.feed_shards)
  const pool = (await Promise.all(Array.from({ length: n }, (_, i) => shard(i)))).flat()
  const terms = await interests(q, pool)
  const seed = Math.abs(q.seed ?? 0) % 2 ** 31
  const perBlog = new Map<number, number>()
  const scored: { p: PostCard; score: number }[] = []
  for (const p of pool.filter(postFilter(q))) {
    const rn = (perBlog.get(p.blog.id) ?? 0) + 1
    perBlog.set(p.blog.id, rn)
    if (rn > 8) continue
    const score =
      p.sort_at / 86400 + noise(p.id, seed) * 14 - (rn - 1) * 5 + similarity(p, terms) * INTEREST_BOOST_DAYS
    scored.push({ p, score: p.undated ? score - 1e9 : score })
  }
  scored.sort((a, b) => b.score - a.score || b.p.id - a.p.id)
  return page(
    scored.map((s) => s.p),
    q.cursor,
    limit,
  )
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
  const newestFirst = q.mode === 'latest' || q.mode === 'following' || !!search || !!q.category || !!q.blogs
  if (!newestFirst) return discover(q, limit)

  const filter = postFilter(q)
  return scan(
    (p) =>
      filter(p) &&
      (!search ||
        lower(p.title).includes(search) ||
        lower(p.excerpt).includes(search) ||
        lower(p.blog.name).includes(search)),
    q.cursor,
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
  if (b.topics.some((t) => t.label.includes(needle) || needle.includes(t.label))) return 5
  return null
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
    .filter((b) => !q.tag || b.tags.some((t) => t.tag === q.tag))
    .filter((b) => !q.topic || b.topics.some((t) => t.id === q.topic))
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
  posts,
  post,
  related,
  blogs,
  blog: async (id: number) => (await blogFile(id)).blog,
}
