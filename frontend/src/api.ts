import { shallowRef } from 'vue'
import { staticApi } from './staticApi'
import type { BlogDetail, BlogSort, BlogSummary, FeedMode, Meta, Page, PostCard, PostDetail, TagCount, TagsInfo } from './types'

type Params = Record<string, string | number | null | undefined>

async function get<T>(path: string, params: Params = {}): Promise<T> {
  const url = new URL(path, location.origin)
  for (const [k, v] of Object.entries(params)) {
    if (v !== null && v !== undefined && v !== '') url.searchParams.set(k, String(v))
  }
  const res = await fetch(url)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return (await res.json()) as T
}

export interface PostQuery {
  /** `following` is `latest` limited to `blogs`. */
  mode?: FeedMode
  cursor?: string | null
  seed?: number
  blog?: number
  q?: string
  limit?: number
  /** Directory tag of the blog (开往: tech, life, ...). */
  tag?: string
  /** A tag of the post: its own category or one derived from its keywords. */
  category?: string
  /** Content area id (dev, travel, ...). */
  topic?: string
  /** Source kind id (blog, news, podcast, ...). */
  kind?: string
  /** Only these blogs. */
  blogs?: number[]
  /** Discover leans towards what these blogs write about ... */
  follow?: number[]
  /** ... and towards posts like these. */
  read?: number[]
  /** Only posts with a cover image. */
  cover?: boolean
}

export interface BlogQuery {
  offset?: number
  limit?: number
  q?: string
  tag?: string
  topic?: string
  kind?: string
  /** Only these blogs. */
  ids?: number[]
  sort?: BlogSort
}

const list = (ids?: number[]) => (ids?.length ? ids.join(',') : undefined)

/** Built for static hosting (GitHub Pages): data comes from exported JSON files. */
export const isStatic = import.meta.env.VITE_STATIC === '1'

const liveApi = {
  meta: () => get<Meta>('/api/meta'),
  tags: () => get<TagsInfo>('/api/tags'),
  posts: ({ cover, blogs, follow, read, mode, ...q }: PostQuery) =>
    get<Page<PostCard>>('/api/posts', {
      ...q,
      mode: mode === 'following' ? 'latest' : mode,
      cover: cover ? 1 : undefined,
      blogs: list(blogs),
      follow: list(follow),
      read: list(read),
    }),
  post: (id: number) => get<PostDetail>(`/api/posts/${id}`),
  related: (id: number, limit = 8) => get<PostCard[]>(`/api/posts/${id}/related`, { limit }),
  blogs: ({ ids, ...q }: BlogQuery) => get<Page<BlogSummary>>('/api/blogs', { ...q, ids: list(ids) }),
  blog: (id: number) => get<BlogDetail>(`/api/blogs/${id}`),
}

export const api: typeof liveApi = isStatic ? staticApi : liveApi

/** Cards already on screen, so the detail view can render instantly. Bounded. */
const seenPosts = new Map<number, PostCard>()
const SEEN_LIMIT = 2000
export function rememberPosts(items: PostCard[]) {
  for (const p of items) seenPosts.set(p.id, p)
  // Maps iterate in insertion order: drop the oldest.
  for (const id of seenPosts.keys()) {
    if (seenPosts.size <= SEEN_LIMIT) break
    seenPosts.delete(id)
  }
}
export function cachedPost(id: number): PostCard | undefined {
  return seenPosts.get(id)
}

export const meta = shallowRef<Meta | null>(null)
export function loadMeta() {
  api
    .meta()
    .then((m) => {
      meta.value = m
      document.title = m.site_name
    })
    .catch(() => {})
}

export const tagsInfo = shallowRef<TagsInfo | null>(null)
export function loadTags() {
  api
    .tags()
    .then((t) => (tagsInfo.value = t))
    .catch(() => {})
}

/** Blog profiles, shared by the post view's about panel and the blog page. */
const blogCache = new Map<number, Promise<BlogDetail>>()
export function blogDetail(id: number): Promise<BlogDetail> {
  let p = blogCache.get(id)
  if (!p) {
    p = api.blog(id)
    p.catch(() => blogCache.delete(id))
    blogCache.set(id, p)
    if (blogCache.size > 200) blogCache.delete(blogCache.keys().next().value!)
  }
  return p
}

export function topicLabel(id: string): string {
  return tagsInfo.value?.topics.find((t) => t.id === id)?.label ?? id
}

export function kindLabel(id: string): string {
  return tagsInfo.value?.kinds.find((k) => k.id === id)?.label ?? KIND_LABELS[id] ?? id
}

const KIND_LABELS: Record<string, string> = {
  blog: '博客',
  news: '资讯',
  newsletter: '周刊',
  podcast: '播客',
  video: '视频',
  forum: '论坛',
  other: '其他',
}

/** Directory tags across all directories, merged by tag name, busiest first. */
export function channelTags(info: TagsInfo | null): TagCount[] {
  const merged = new Map<string, TagCount>()
  for (const d of info?.directories ?? []) {
    for (const t of d.tags) {
      const prev = merged.get(t.tag)
      merged.set(t.tag, prev ? { ...prev, blogs: prev.blogs + t.blogs, posts: prev.posts + t.posts } : { ...t })
    }
  }
  return [...merged.values()].sort((a, b) => b.blogs - a.blogs)
}

export function tagLabel(tag: string): string {
  return channelTags(tagsInfo.value).find((t) => t.tag === tag)?.label ?? tag
}
