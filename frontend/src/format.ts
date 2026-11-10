export function relativeTime(unixSecs: number | null | undefined): string {
  if (!unixSecs) return ''
  const diff = Date.now() / 1000 - unixSecs
  if (diff < 60) return '刚刚'
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`
  if (diff < 7 * 86400) return `${Math.floor(diff / 86400)} 天前`
  const d = new Date(unixSecs * 1000)
  const md = `${d.getMonth() + 1}-${d.getDate()}`
  return d.getFullYear() === new Date().getFullYear() ? md : `${d.getFullYear()}-${md}`
}

export function fullDate(unixSecs: number | null | undefined): string {
  if (!unixSecs) return ''
  return new Date(unixSecs * 1000).toLocaleString('zh-CN', { dateStyle: 'long', timeStyle: 'short' })
}

/** Stable 0..359 hue for a string, used for avatars and text covers. */
export function hue(seed: string | number): number {
  const s = String(seed)
  let h = 2166136261
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 16777619)
  }
  return (h >>> 0) % 360
}

export function initial(name: string): string {
  const ch = Array.from(name.trim())[0] ?? '?'
  return ch.toUpperCase()
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, '')
  } catch {
    return url
  }
}

// Card geometry, shared by the card and the waterfall's height estimates.
// Ratios are height / width.
export const COVER_MIN = 0.6
export const COVER_MAX = 4 / 3
export const COVER_DEFAULT = 0.75
const TEXT_COVER_RATIOS = [1, 1.2, 4 / 3]

/** Shorter excerpts are hardly anything: cards show the title instead, and
 *  「有摘要」 leaves them out (as the server's `text=1` does). */
export const MIN_TEXT_CHARS = 20

export const hasText = (post: { excerpt: string }) => [...post.excerpt.trim()].length >= MIN_TEXT_CHARS

/** What a card without a picture shows: the excerpt, or the title when the
 *  excerpt is hardly anything. */
export function textCoverText(post: { title: string; excerpt: string }): string {
  return hasText(post) ? post.excerpt.trim() : post.title
}

/** A card without a picture: one of a few shapes for variety, but no taller
 *  than its text needs, so a line or two makes a small card. Measured for a
 *  column about 230px wide: some 11 CJK characters (20 Latin) per line, at
 *  most 6 lines shown. Narrower columns fit less; the card grows to fit. */
export function textCoverRatio(post: { id: number; title: string; excerpt: string }): number {
  let width = 0
  for (const ch of textCoverText(post)) width += ch.charCodeAt(0) > 0x2e7f ? 1 : 0.55
  const lines = Math.min(6, Math.max(1, Math.ceil(width / 11)))
  return Math.min(TEXT_COVER_RATIOS[post.id % TEXT_COVER_RATIOS.length], 0.3 + lines * 0.115)
}

export function clampCover(ratio: number): number {
  return Math.min(COVER_MAX, Math.max(COVER_MIN, ratio))
}
