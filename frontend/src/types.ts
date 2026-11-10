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
  /** Whether its pages may be shown in a frame here (the reader pane); null = not known yet. */
  framable?: boolean | null
}

/**
 * A category: top-level (`travel`), a sub-category (`tech/ai`, with its
 * parent), or `other`. Every post has exactly one.
 */
export interface CategoryRef {
  id: string
  label: string
  parent?: { id: string; label: string }
  /** Blogs only: share of the blog's classified posts. */
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
  /** The author's own tags and categories. */
  tags: string[]
  /** Tags derived from the post's keywords. */
  auto_tags?: string[]
  category: CategoryRef
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
  /** Who runs the site, and their blog here if it is one of ours. */
  maintainer?: { name: string; url: string | null; blog: { id: number; name: string } | null } | null
  /** Waterfall columns on the widest screens (`[server] columns`). */
  columns?: number
  /** Where bloggers ask for changes: the source repository and its config file. */
  repo?: { url: string; config_path: string } | null
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
  /** Main top-level categories. */
  categories: CategoryRef[]
  framable?: boolean | null
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
  /** The whole about page (sanitised HTML), when the author asked for it. */
  about_html?: string | null
  comments_embedded?: boolean
  /** How crawling goes, for the blog's author to check (absent from older servers). */
  crawl?: CrawlStatus
}

export interface CrawlStatus {
  feed_url: string | null
  /** `directory` (listed there), `discovered` (found on the site) or `manual` (set in the config). */
  feed_origin: string | null
  last_checked_at: number | null
  last_success_at: number | null
  next_check_at: number
  failures: number
  last_error: string | null
  /** Last search for a feed, for blogs without one. */
  discovery_at: number | null
  /** When the homepage and about page were last read. */
  profile_at: number | null
  found_avatar: string | null
  feed_image: string | null
  about_page: string | null
  /** Settings from the site's config: avatar, description, social, tags, about, feed, full_text, comments. */
  configured: string[]
}

export interface TagCount {
  tag: string
  label: string
  blogs: number
  posts: number
}

/** A node of the category tree; `children` are sub-categories. */
export interface CategoryNode {
  id: string
  label: string
  /** Top-level only: blogs whose main categories include it. */
  blogs?: number
  posts: number
  children?: CategoryNode[]
}

export interface TagStat {
  name: string
  blogs: number
  posts: number
}

export interface TagsInfo {
  directories: { id: string; name: string; tags: TagCount[] }[]
  /** The category tree, in channel order. */
  categories: CategoryNode[]
  /** Source kinds present. */
  kinds: { id: string; label: string; blogs: number }[]
  /** Popular tags, recent months. */
  popular_tags: TagStat[]
}

export type BlogSort = 'recent' | 'posts' | 'name'

/** `explore` leans away from the reader's interests, `discover` (with signals) towards them. */
export type FeedMode = 'discover' | 'explore' | 'latest'
