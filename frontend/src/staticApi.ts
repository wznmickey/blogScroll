// Static mode (VITE_STATIC=1): the site is plain files, e.g. on GitHub Pages.
// Same functions as the live API, answered in the browser from the JSON that
// `feed-server export` writes to <base>/data/. See backend/src/export.rs.

import type { BlogQuery, PostQuery } from './api'
import type { BlogDetail, BlogSummary, Meta, Page, PostCard, PostDetail, TagsInfo } from './types'

const DATA = `${import.meta.env.BASE_URL}data/`
/** Shards (newest first) that "推荐" mixes from. */
const DISCOVER_SHARDS = 2

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

function page<T>(all: T[], cursor: string | null | undefined, limit: number): Page<T> {
  const offset = Math.max(0, Number(cursor) || 0)
  return {
    items: all.slice(offset, offset + limit),
    next_cursor: offset + limit < all.length ? String(offset + limit) : null,
  }
}

function blogFilter(q: PostQuery) {
  return (p: PostCard) => (!q.tag || (p.blog.tags ?? []).includes(q.tag)) && (!q.cover || !!p.cover)
}

/** Same scoring as the server's discover mode (backend/src/api.rs). */
function noise(id: number, seed: number) {
  return ((id * 2654435761 + seed) % 1000003) / 1000003
}

async function discover(q: PostQuery, limit: number): Promise<Page<PostCard>> {
  const idx = await index()
  const n = Math.min(DISCOVER_SHARDS, idx.feed_shards)
  const pool = (await Promise.all(Array.from({ length: n }, (_, i) => shard(i)))).flat().filter(blogFilter(q))
  const seed = Math.abs(q.seed ?? 0) % 2 ** 31
  const perBlog = new Map<number, number>()
  const scored: { p: PostCard; score: number }[] = []
  for (const p of pool) {
    const rn = (perBlog.get(p.blog.id) ?? 0) + 1
    perBlog.set(p.blog.id, rn)
    if (rn > 8) continue
    scored.push({ p, score: p.sort_at / 86400 + noise(p.id, seed) * 14 - (rn - 1) * 5 })
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
  const category = q.category?.trim().toLowerCase()
  if (q.mode !== 'latest' && !search && !category) return discover(q, limit)

  const byBlog = blogFilter(q)
  return scan(
    (p) =>
      byBlog(p) &&
      (!category || p.tags.some((t) => t.toLowerCase() === category)) &&
      (!search ||
        p.title.toLowerCase().includes(search) ||
        p.excerpt.toLowerCase().includes(search) ||
        p.blog.name.toLowerCase().includes(search)),
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

async function blogs(q: BlogQuery): Promise<Page<BlogSummary>> {
  const needle = q.q?.trim().toLowerCase()
  const recent = (a: BlogSummary, b: BlogSummary) =>
    (b.last_post_at ?? -1) - (a.last_post_at ?? -1) || a.id - b.id
  const sorters = {
    recent,
    posts: (a: BlogSummary, b: BlogSummary) => b.post_count - a.post_count || recent(a, b),
    name: (a: BlogSummary, b: BlogSummary) => a.name.localeCompare(b.name, 'zh-CN'),
  }
  const all = (await file<BlogSummary[]>('blogs.json'))
    .filter((b) => !q.tag || b.tags.some((t) => t.tag === q.tag))
    .filter((b) => !needle || b.name.toLowerCase().includes(needle) || b.homepage.toLowerCase().includes(needle))
    .sort(sorters[q.sort ?? 'recent'])
  return page(all, String(q.offset ?? 0), q.limit ?? 48)
}

export const staticApi = {
  meta: () => file<Meta>('meta.json'),
  tags: () => file<TagsInfo>('tags.json'),
  posts,
  post,
  blogs,
  blog: async (id: number) => (await blogFile(id)).blog,
}
