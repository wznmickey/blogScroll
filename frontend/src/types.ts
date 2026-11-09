// Mirrors the JSON returned by backend/src/api.rs.

export interface BlogRef {
  id: number
  name: string
  homepage: string
  icon: string | null
}

export interface PostCard {
  id: number
  title: string
  url: string
  excerpt: string
  cover: string | null
  author: string | null
  tags: string[]
  /** Unix seconds. */
  published_at: number | null
  sort_at: number
  has_full_text: boolean
  blog: BlogRef
}

export interface PostDetail extends PostCard {
  /** Sanitised HTML, only for blogs whose author allowed full text. */
  content_html: string | null
}

export interface Page<T> {
  items: T[]
  next_cursor: string | null
}

export interface DirectoryInfo {
  id: string
  name: string
  homepage: string
}

export interface Meta {
  site_name: string
  directories: DirectoryInfo[]
  stats: {
    blogs: number
    blogs_with_feed: number
    posts: number
    last_directory_sync: number | null
  }
}

export interface BlogDetail {
  id: number
  name: string
  homepage: string
  icon: string | null
  post_count: number
  last_post_at: number | null
  has_feed: boolean
  full_text: boolean
  directories: { directory: string; name: string; tags: string[] }[]
}

export type FeedMode = 'discover' | 'latest'
