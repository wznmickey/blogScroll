import { shallowRef } from 'vue'
import type { Hide } from './channels'
import { WEIGHT_INFO, type Weights } from './options'
import { crawledIds, ownBlog, ownDetail, ownPost, ownPosts } from './sources'
import { staticApi } from './staticApi'
import type {
  BlogDetail,
  BlogSort,
  BlogSummary,
  CategoryNode,
  CategoryRef,
  FeedMode,
  InterestProfile,
  Meta,
  Page,
  PostCard,
  PostDetail,
  TagCount,
  TagStat,
  TagsInfo,
  Trending,
} from './types'

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
  mode?: FeedMode
  cursor?: string | null
  seed?: number
  blog?: number
  /** Title, excerpt, blog name, or exactly a tag. */
  q?: string
  limit?: number
  /** Directory tag of the blog (开往: tech, life, ...). */
  dirtag?: string
  /** A tag of the post: the author's own or one derived from its keywords. */
  tag?: string
  /** `tech` (with its sub-categories) or `tech/ai`. */
  category?: string
  /** Source kind id (blog, news, podcast, ...). */
  kind?: string
  /** Only these blogs. */
  blogs?: number[]
  /** Personalised discover: followed blogs, and liked, saved and opened posts. */
  follow?: number[]
  like?: number[]
  save?: number[]
  read?: number[]
  /** Shown lately but not opened: passed over a little. */
  shown?: number[]
  /** Newest-first feeds: start at this post (inclusive) instead of the top. */
  startAt?: { sort_at: number; id: number }
  /** Posts to leave out. */
  hide?: Hide
  /** What the reader blocked (settings): blogs, sites, words or tags. */
  blockBlogs?: number[]
  blockHosts?: string[]
  blockWords?: string[]
  /** Also adult content (R18 / NSFW). */
  nsfw?: boolean
  /** Only blogs from these directories. */
  directories?: string[]
  /** The reader's own weights for 推荐 / 反推荐. */
  weights?: Weights
}

/** The signals 推荐 reads, for its picture of the reader (settings). */
export interface InterestQuery {
  follow?: number[]
  like?: number[]
  save?: number[]
  read?: number[]
}

export interface BlogQuery {
  offset?: number
  limit?: number
  q?: string
  dirtag?: string
  /** A top-level category. */
  category?: string
  kind?: string
  /** Only these blogs. */
  ids?: number[]
  sort?: BlogSort
}

/** The site's ids only: the reader's own feeds' (negative) mean nothing to the server. */
const list = (ids?: number[]) => {
  const ours = ids?.filter((id) => id > 0)
  return ours?.length ? ours.join(',') : undefined
}

// A server from before categories (until it is restarted on the new version)
// sends `topics` instead of `category`, and its popular tags as `categories`.
const OTHER: CategoryRef = { id: 'other', label: '其他' }
const withCategory = <T extends PostCard>(p: T): T => (p.category ? p : { ...p, category: OTHER })
function upgradeTags(t: TagsInfo): TagsInfo {
  const old = (t.categories as unknown as { id?: string }[]).some((c) => !c.id)
  return old
    ? { ...t, categories: [], popular_tags: t.categories as unknown as TagStat[] }
    : { ...t, popular_tags: t.popular_tags ?? [] }
}

/** Built for static hosting (GitHub Pages): data comes from exported JSON files. */
export const isStatic = import.meta.env.VITE_STATIC === '1'

const liveApi = {
  meta: () => get<Meta>('/api/meta'),
  /** 大家在看: what other readers took to lately. */
  trending: () => get<Trending>('/api/trending'),
  tags: () => get<TagsInfo>('/api/tags'),
  /** Tags whose name contains `q`, most used first. */
  searchTags: (q: string, limit = 20) => get<TagStat[]>('/api/tags/search', { q, limit }),
  posts: async ({
    hide,
    blogs,
    follow,
    like,
    save,
    read,
    shown,
    startAt,
    cursor,
    blockBlogs,
    blockHosts,
    blockWords,
    nsfw,
    directories,
    weights,
    ...q
  }: PostQuery) => {
    const page = await get<Page<PostCard>>('/api/posts', {
      ...q,
      // The cursor is exclusive: one past the post itself.
      cursor: cursor ?? (startAt ? `${startAt.sort_at}.${startAt.id + 1}` : undefined),
      cover: hide?.noCover ? 1 : undefined,
      text: hide?.shortExcerpt ? 1 : undefined,
      comments: hide?.noComments ? 1 : undefined,
      block_blogs: list(blockBlogs),
      block_hosts: blockHosts?.length ? blockHosts.join(',') : undefined,
      block_words: blockWords?.length ? blockWords.join(',') : undefined,
      nsfw: nsfw ? 1 : undefined,
      directories: directories?.join(','),
      // The reader's own feeds that the site crawls: listed for them.
      own: list(crawledIds()),
      w: weights ? WEIGHT_INFO.map(({ key }) => weights[key]).join(',') : undefined,
      blogs: list(blogs),
      follow: list(follow),
      like: list(like),
      save: list(save),
      read: list(read),
      shown: list(shown),
    })
    return { ...page, items: page.items.map(withCategory) }
  },
  post: async (id: number) => withCategory(await get<PostDetail>(`/api/posts/${id}`)),
  interest: ({ follow, like, save, read }: InterestQuery) =>
    get<InterestProfile>('/api/interest', { follow: list(follow), like: list(like), save: list(save), read: list(read) }),
  related: async (id: number, limit = 8) =>
    (await get<PostCard[]>(`/api/posts/${id}/related`, { limit })).map(withCategory),
  blogs: ({ ids, ...q }: BlogQuery) => get<Page<BlogSummary>>('/api/blogs', { ...q, ids: list(ids) }),
  blog: (id: number) => get<BlogDetail>(`/api/blogs/${id}`),
  /** By its address, as in /blog/{key}/. */
  blogByKey: (key: string) => get<BlogDetail>(`/api/sites/${key}`),
}

const siteApi: typeof liveApi = isStatic ? staticApi : liveApi

const notFound = () => Promise.reject(new Error('HTTP 404'))

/** The site's API, with the reader's own feeds (negative ids, sources.ts) answered here. */
export const api: typeof liveApi = {
  ...siteApi,
  posts: (q) =>
    q.blog !== undefined && q.blog < 0
      ? Promise.resolve(ownPosts({ blog: q.blog, cursor: q.cursor, limit: q.limit }))
      : siteApi.posts(q),
  post: (id) => (id < 0 ? (ownDetail(id) ? Promise.resolve(ownDetail(id)!) : notFound()) : siteApi.post(id)),
  related: (id, limit) => (id < 0 ? Promise.resolve([]) : siteApi.related(id, limit)),
  blog: (id) => (id < 0 ? (ownBlog(id) ? Promise.resolve(ownBlog(id)!) : notFound()) : siteApi.blog(id)),
  blogByKey: (key) => (/^-\d+$/.test(key) ? api.blog(Number(key)) : siteApi.blogByKey(key)),
}

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
  return seenPosts.get(id) ?? (id < 0 ? ownPost(id) : undefined)
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
    .then((t) => (tagsInfo.value = upgradeTags(t)))
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

/** Every category, top-level ones followed by their sub-categories. */
export function allCategories(info: TagsInfo | null = tagsInfo.value): CategoryNode[] {
  return (info?.categories ?? []).flatMap((c) => [c, ...(c.children ?? [])])
}

export function findCategory(id: string): CategoryNode | undefined {
  return allCategories().find((c) => c.id === id)
}

/** "技术 › AI", "旅行". */
export function categoryName(c: CategoryRef | string): string {
  if (typeof c !== 'string') return c.parent ? `${c.parent.label} › ${c.label}` : c.label
  const [top, sub] = c.split('/')
  const parent = tagsInfo.value?.categories.find((x) => x.id === top)
  if (!sub) return parent?.label ?? c
  const child = parent?.children?.find((x) => x.id === c)
  return child ? `${parent!.label} › ${child.label}` : c
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
