// Mirrors the JSON returned by backend/src/api.rs.

export interface BlogRef {
  id: number
  name: string
  homepage: string
  icon: string | null
  /** Directory tags of the blog (tech, life, ...). */
  tags?: string[]
  /** Source kind id: blog, news, newsletter, podcast, video, forum, other. */
  kind?: string
}

/** A content area (编程开发, 旅行, ...) computed from posts. */
export interface TopicRef {
  id: string
  label: string
  /** Blogs only: share of the blog's classified posts in this area. */
  share?: number
}

export interface KindRef {
  id: string
  label: string
}

export interface PostCard {
  id: number
  title: string
  url: string
  excerpt: string
  cover: string | null
  author: string | null
  /** The author's own categories. */
  tags: string[]
  /** Tags derived from the post's keywords. */
  auto_tags?: string[]
  topics?: TopicRef[]
  /** Unix seconds. */
  published_at: number | null
  sort_at: number
  /** The feed gave no date and the post came with the first fetch: age unknown. */
  undated?: boolean
  has_full_text: boolean
  /** From the feed (slash:comments) or the blog's comment service. */
  comment_count: number | null
  comments_url: string | null
  blog: BlogRef
}

/** How to load a blog's own comment widget for one post (authorized blogs only). */
export type EmbedConfig = { path: string; term: string | null; url: string; title: string } & (
  | { kind: 'twikoo'; env_id: string }
  | { kind: 'waline'; server_url: string }
  | { kind: 'artalk'; server: string; site: string | null }
  | {
      kind: 'giscus'
      repo: string
      repo_id: string | null
      category: string | null
      category_id: string | null
      mapping: string | null
    }
  | { kind: 'utterances'; repo: string; issue_term: string | null }
)

export interface PostDetail extends PostCard {
  /** Sanitised HTML, only for blogs whose author allowed full text. */
  content_html: string | null
  /** Absent in static mode. */
  comments?: { system: string; embed: EmbedConfig | null } | null
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
  /** When the server (or the static export) produced this. */
  generated_at: number
}

/** A directory tag on a blog, e.g. 开往 `tech` → 技术. */
export interface TagRef {
  directory: string
  tag: string
  label: string
}

export interface BlogSummary {
  id: number
  name: string
  homepage: string
  icon: string | null
  post_count: number
  last_post_at: number | null
  has_feed: boolean
  /** The blog's own meta description. */
  description: string | null
  tags: TagRef[]
  kind: KindRef
  topics: TopicRef[]
}

export interface SocialLink {
  kind: string
  url: string
}

export interface BlogDetail extends BlogSummary {
  full_text: boolean
  directories: { directory: string; name: string }[]
  about_url: string | null
  about_text: string | null
  social: SocialLink[]
  generator: string | null
  comment_system: string | null
  /** Tags the blog uses most. */
  top_tags: string[]
}

export interface TagCount {
  tag: string
  label: string
  blogs: number
  posts: number
}

export interface TopicCount {
  id: string
  label: string
  blogs: number
  posts: number
}

export interface TagsInfo {
  directories: { id: string; name: string; tags: TagCount[] }[]
  /** Content areas, busiest first. */
  topics: TopicCount[]
  /** Source kinds present. */
  kinds: { id: string; label: string; blogs: number }[]
  /** Popular categories from the posts' own feeds, recent months. */
  categories: { name: string; blogs: number; posts: number }[]
}

export type BlogSort = 'recent' | 'posts' | 'name'

export type FeedMode = 'discover' | 'latest' | 'following'
