// The reader's settings beyond the channel bar (channels.ts): how 推荐 weighs
// things, what they never want to see, adult content, which directories,
// how many pins, text size. Kept in this browser; included in backups.

import { ref, watch } from 'vue'

/** 推荐's weights, in days of freshness (as on the server, `Weights` in api.rs). */
export interface Weights {
  interest: number
  category: number
  blog: number
  seen: number
  shown: number
  noise: number
  sameBlog: number
  perBlog: number
  /** What other readers took to (heat), days ahead; counts only with `useOthers`. */
  heat: number
}

export const DEFAULT_WEIGHTS: Weights = {
  interest: 40,
  category: 12,
  blog: 5,
  seen: 60,
  shown: 10,
  noise: 14,
  sameBlog: 5,
  perBlog: 8,
  heat: 10,
}

/** In the order the server reads them (`w=`), with the settings page's wording. */
export const WEIGHT_INFO: { key: keyof Weights; label: string; hint: string; max: number; step: number }[] = [
  { key: 'interest', label: '兴趣匹配', hint: '和你喜欢、收藏、读过的文章越像，最多往前提多少天', max: 100, step: 1 },
  { key: 'category', label: '常看的分类', hint: '你常看的分类里的文章往前提多少天', max: 50, step: 1 },
  { key: 'blog', label: '关注的博主', hint: '关注的博主的文章往前提多少天', max: 50, step: 1 },
  { key: 'seen', label: '已经看过', hint: '喜欢、收藏或打开过的文章往后放多少天', max: 365, step: 5 },
  { key: 'shown', label: '刷到过没点', hint: '最近刷到但没打开的文章往后放多少天', max: 60, step: 1 },
  { key: 'noise', label: '随机打散', hint: '每篇随机加 0 到这么多天，同一时间的文章才不会总是一样的顺序', max: 60, step: 1 },
  { key: 'sameBlog', label: '同一博主', hint: '同一个博主每多一篇，往后放多少天，免得一个人刷屏', max: 30, step: 1 },
  { key: 'perBlog', label: '每位博主最多', hint: '每个博主最多出现几篇（反推荐最多 3 篇）', max: 30, step: 1 },
  { key: 'heat', label: '其他读者在看', hint: '其他读者打开、喜欢、收藏、读得久的文章往前提多少天（反推荐往后放）', max: 60, step: 1 },
]

export interface BlockedBlog {
  id: number
  name: string
}

export interface Options {
  weights: Weights
  /** Channels kept at the left of the bar. */
  maxPinned: number
  /** Text (and everything with it) this many times the usual size. */
  textSize: number
  /** Show posts marked R18 / NSFW. */
  showNsfw: boolean
  blockBlogs: BlockedBlog[]
  /** Sites, e.g. `example.com` (its subdomains too). */
  blockHosts: string[]
  /** Words or tags, in a title, an excerpt or among a post's tags. */
  blockWords: string[]
  /** Only blogs from these directories; null = all of them. */
  directories: string[] | null
  /** Send anonymous counts of what one does here (telemetry.ts). */
  sendStats: boolean
  /** 推荐 counts what other readers took to (the `heat` weight). */
  useOthers: boolean
  /** Show posts' 热度 on cards and in posts. */
  showHeat: boolean
}

export const TEXT_SIZES = [
  { value: 0.9, label: '小' },
  { value: 1, label: '标准' },
  { value: 1.1, label: '大' },
  { value: 1.25, label: '特大' },
]
export const PIN_CHOICES = [1, 2, 3, 4, 5, 6]

const KEY = 'options'

const num = (v: unknown, lo: number, hi: number, fallback: number) =>
  typeof v === 'number' && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : fallback
const strings = (v: unknown, max: number) =>
  Array.isArray(v)
    ? [...new Set(v.filter((s): s is string => typeof s === 'string' && !!s.trim() && s.length <= 100).map((s) => s.trim()))].slice(0, max)
    : []

export function cleanOptions(v: unknown): Options {
  const o = (v ?? {}) as Partial<Options>
  const w = (o.weights ?? {}) as Partial<Weights>
  const weights = Object.fromEntries(
    WEIGHT_INFO.map(({ key, max }) => [key, num(w[key], key === 'perBlog' ? 1 : 0, max, DEFAULT_WEIGHTS[key])]),
  ) as unknown as Weights
  weights.perBlog = Math.round(weights.perBlog)
  return {
    weights,
    maxPinned: Math.round(num(o.maxPinned, 1, 6, 3)),
    textSize: TEXT_SIZES.some((t) => t.value === o.textSize) ? (o.textSize as number) : 1,
    showNsfw: o.showNsfw === true,
    blockBlogs: Array.isArray(o.blockBlogs)
      ? o.blockBlogs
          .filter((b): b is BlockedBlog => Number.isInteger(b?.id) && typeof b?.name === 'string')
          .map((b) => ({ id: b.id, name: b.name.slice(0, 100) }))
          .slice(0, 1000)
      : [],
    blockHosts: strings(o.blockHosts, 200).map(hostKey).filter(Boolean),
    blockWords: strings(o.blockWords, 100),
    directories: o.directories == null ? null : strings(o.directories, 20),
    sendStats: o.sendStats !== false,
    useOthers: o.useOthers !== false,
    showHeat: o.showHeat !== false,
  }
}

/** `https://www.Example.com/x` → `example.com` (as the server compares them). */
export function hostKey(s: string): string {
  const rest = s.trim().toLowerCase().replace(/^[a-z]+:\/\//, '')
  return rest.split(/[/?#]/)[0].replace(/^www\./, '')
}

function load(): Options {
  try {
    return cleanOptions(JSON.parse(localStorage.getItem(KEY) ?? 'null'))
  } catch {
    return cleanOptions(null)
  }
}

export const options = ref<Options>(load())

watch(
  options,
  (o) => {
    try {
      localStorage.setItem(KEY, JSON.stringify(o))
    } catch {
      // Applies to this visit only.
    }
  },
  { deep: true },
)

window.addEventListener('storage', (e) => {
  if (e.key === KEY) options.value = load()
})

// Text size: the page and an open post scale with it (style.css).
watch(
  () => options.value.textSize,
  (z) => document.documentElement.style.setProperty('--zoom', String(z)),
  { immediate: true },
)

export const isBlocked = (blogId: number) => options.value.blockBlogs.some((b) => b.id === blogId)

export function toggleBlock(blog: BlockedBlog) {
  const list = options.value.blockBlogs
  options.value.blockBlogs = isBlocked(blog.id) ? list.filter((b) => b.id !== blog.id) : [...list, blog]
}

export const sameWeights = (a: Weights, b: Weights) => WEIGHT_INFO.every(({ key }) => a[key] === b[key])

/** What the feeds send for these settings (left out when they say nothing). */
export function readerFilters() {
  const o = options.value
  return {
    blockBlogs: o.blockBlogs.length ? o.blockBlogs.map((b) => b.id) : undefined,
    blockHosts: o.blockHosts.length ? [...o.blockHosts] : undefined,
    blockWords: o.blockWords.length ? [...o.blockWords] : undefined,
    nsfw: o.showNsfw || undefined,
    directories: o.directories ? [...o.directories] : undefined,
  }
}

/** 推荐's weights as the reader set them (left out when they are the default); others' reading only if they want it. */
export function weightsFor() {
  const w = { ...options.value.weights, heat: options.value.useOthers ? options.value.weights.heat : 0 }
  return sameWeights(w, DEFAULT_WEIGHTS) ? undefined : w
}
