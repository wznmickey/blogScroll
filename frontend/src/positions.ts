// Where the reader got to in each newest-first feed (最新, 关注, a category
// sorted by time…): the newest and the oldest post shown to them so far. On
// the next visit, what is newer than the newest counts as new, and reading
// can resume from the oldest. Kept in this browser; included in backups.

export interface Mark {
  sort_at: number
  id: number
}

export interface Position {
  newest: Mark
  oldest: Mark & { title: string }
  /** Last updated, ms since the epoch. */
  at: number
}

const KEY = 'positions'
const MAX_FEEDS = 50

/** Newer first: positive when `a` comes before `b` in a newest-first feed. */
export function compareMarks(a: Mark, b: Mark): number {
  return a.sort_at - b.sort_at || a.id - b.id
}

function isMark(v: unknown): v is Mark {
  const m = v as Mark
  return Number.isFinite(m?.sort_at) && Number.isInteger(m?.id)
}

/** Accepts positions from storage or a backup file, dropping anything malformed. */
export function cleanPositions(v: unknown): Record<string, Position> {
  const out: Record<string, Position> = {}
  if (!v || typeof v !== 'object') return out
  for (const [k, p] of Object.entries(v as Record<string, Position>)) {
    if (isMark(p?.newest) && isMark(p?.oldest) && Number.isFinite(p.at)) {
      out[k] = {
        newest: { sort_at: p.newest.sort_at, id: p.newest.id },
        oldest: { sort_at: p.oldest.sort_at, id: p.oldest.id, title: String(p.oldest.title ?? '').slice(0, 200) },
        at: p.at,
      }
    }
  }
  return out
}

function load(): Record<string, Position> {
  try {
    return cleanPositions(JSON.parse(localStorage.getItem(KEY) ?? '{}'))
  } catch {
    return {}
  }
}

let positions = load()
let saveTimer: ReturnType<typeof setTimeout> | undefined

function save() {
  clearTimeout(saveTimer)
  saveTimer = undefined
  // Only the most recently used feeds are kept.
  const keep = Object.entries(positions)
    .sort((a, b) => b[1].at - a[1].at)
    .slice(0, MAX_FEEDS)
  positions = Object.fromEntries(keep)
  try {
    localStorage.setItem(KEY, JSON.stringify(positions))
  } catch {
    // Kept for this visit only.
  }
}

window.addEventListener('pagehide', save)
window.addEventListener('storage', (e) => {
  if (e.key === KEY) positions = load()
})

export function lastPosition(feed: string): Position | undefined {
  const p = positions[feed]
  return p && { newest: { ...p.newest }, oldest: { ...p.oldest }, at: p.at }
}

/** A post of this feed was shown: widen what the reader has seen of it. */
export function notePosition(feed: string, post: Mark & { title: string }) {
  const mark = { sort_at: post.sort_at, id: post.id }
  const p = positions[feed]
  positions[feed] = {
    newest: !p || compareMarks(mark, p.newest) > 0 ? mark : p.newest,
    oldest: !p || compareMarks(mark, p.oldest) < 0 ? { ...mark, title: post.title } : p.oldest,
    at: Date.now(),
  }
  saveTimer ??= setTimeout(save, 1500)
}

export function allPositions(): Record<string, Position> {
  return JSON.parse(JSON.stringify(positions))
}

export function restorePositions(incoming: Record<string, Position>) {
  positions = { ...incoming }
  save()
}
