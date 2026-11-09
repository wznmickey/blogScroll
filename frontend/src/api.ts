import { shallowRef } from 'vue'
import type { BlogDetail, FeedMode, Meta, Page, PostCard, PostDetail } from './types'

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
  q?: string
  limit?: number
}

export const api = {
  meta: () => get<Meta>('/api/meta'),
  posts: (q: PostQuery) => get<Page<PostCard>>('/api/posts', { ...q }),
  post: (id: number) => get<PostDetail>(`/api/posts/${id}`),
  blog: (id: number) => get<BlogDetail>(`/api/blogs/${id}`),
}

/** Cards already on screen, so the detail view can render instantly. */
const seenPosts = new Map<number, PostCard>()
export function rememberPosts(items: PostCard[]) {
  for (const p of items) seenPosts.set(p.id, p)
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
