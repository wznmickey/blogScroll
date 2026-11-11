// The copy of a reader's data that their devices sync (sync.ts), and how two
// copies merge: everything either has, the newer of each entry, and without
// what either removed later than the other added it (removals are kept as
// "removed at" marks for REMOVED_DAYS). Settings objects: the newer one.
// Pure functions, tested on their own (tests/syncDoc.test.ts).

export interface SyncEntry {
  /** Added (or, for 刷到过, last shown), ms since the epoch. */
  at: number
  count?: number
  post: { id: number }
}

export interface SyncMark {
  sort_at: number
  id: number
}

export interface SyncPosition {
  newest: SyncMark
  oldest: SyncMark & { title: string }
  at: number
}

export interface SyncSource {
  url: string
  /** Added or last changed, ms. */
  at: number
}

export interface Stamped<T = unknown> {
  /** Last changed, ms; 0 = not since sync began. */
  at: number
  value: T
}

export interface SyncDoc<E extends SyncEntry = SyncEntry, S extends SyncSource = SyncSource> {
  v: 1
  /** Followed blogs: id → when followed (0 = before sync knew). */
  follows: Record<string, number>
  /** likes, saves, history, seen: newest first. */
  lists: Record<string, E[]>
  positions: Record<string, SyncPosition>
  /** The reader's own feeds and lists, by address. */
  sources: Record<string, S>
  /** channels, options, settings. */
  objects: Record<string, Stamped>
  /** Collection (follows, likes, ..., sources) → key → removed at, ms. */
  removed: Record<string, Record<string, number>>
}

export const REMOVED_DAYS = 120
const MAX_POSITIONS = 50

const keys = (...objs: object[]) => [...new Set(objs.flatMap((o) => Object.keys(o ?? {})))]

/** Merges `local` and `remote`; on a tie (same time), `remote` wins for settings. */
export function mergeDocs<E extends SyncEntry, S extends SyncSource>(
  local: SyncDoc<E, S>,
  remote: SyncDoc<E, S>,
  now = Date.now(),
): SyncDoc<E, S> {
  // Every mark counts now; only the recent ones are kept.
  const marks: Record<string, Record<string, number>> = {}
  const removed: Record<string, Record<string, number>> = {}
  const since = now - REMOVED_DAYS * 86_400_000
  for (const col of keys(local.removed, remote.removed)) {
    const all: Record<string, number> = {}
    for (const src of [local.removed[col], remote.removed[col]]) {
      for (const [k, at] of Object.entries(src ?? {})) if (!(all[k] >= at)) all[k] = at
    }
    marks[col] = all
    const recent = Object.entries(all).filter(([, at]) => at >= since)
    if (recent.length) removed[col] = Object.fromEntries(recent)
  }
  const gone = (col: string, key: string | number, at: number) => (marks[col]?.[String(key)] ?? -1) >= at

  const follows: Record<string, number> = {}
  for (const src of [local.follows, remote.follows]) {
    for (const [id, at] of Object.entries(src ?? {})) if (!(follows[id] >= at)) follows[id] = at
  }
  for (const [id, at] of Object.entries(follows)) if (gone('follows', id, at)) delete follows[id]

  const lists: Record<string, E[]> = {}
  for (const name of keys(local.lists, remote.lists)) {
    const byId = new Map<number, E>()
    for (const e of [...(local.lists[name] ?? []), ...(remote.lists[name] ?? [])]) {
      const prev = byId.get(e.post.id)
      if (!prev) {
        byId.set(e.post.id, e)
        continue
      }
      const newer = e.at > prev.at ? e : prev
      // 刷到过 counts showings: the higher count of the two.
      byId.set(e.post.id, name === 'seen' ? { ...newer, count: Math.max(prev.count ?? 1, e.count ?? 1) } : newer)
    }
    lists[name] = [...byId.values()].filter((e) => !gone(name, e.post.id, e.at)).sort((a, b) => b.at - a.at)
  }

  const positions: Record<string, SyncPosition> = {}
  for (const k of keys(local.positions, remote.positions)) {
    const a = local.positions[k]
    const b = remote.positions[k]
    positions[k] =
      a && b
        ? {
            newest: later(a.newest, b.newest) ? a.newest : b.newest,
            oldest: later(a.oldest, b.oldest) ? b.oldest : a.oldest,
            at: Math.max(a.at, b.at),
          }
        : (a ?? b)
  }
  const kept = Object.entries(positions)
    .sort((x, y) => y[1].at - x[1].at)
    .slice(0, MAX_POSITIONS)

  const sources: Record<string, S> = {}
  for (const url of keys(local.sources, remote.sources)) {
    const a = local.sources[url]
    const b = remote.sources[url]
    const s = a && b ? (a.at > b.at ? a : b) : (a ?? b)
    if (!gone('sources', url, s.at)) sources[url] = s
  }

  const objects: Record<string, Stamped> = {}
  for (const name of keys(local.objects, remote.objects)) {
    const a = local.objects[name]
    const b = remote.objects[name]
    objects[name] = a && b ? (a.at > b.at ? a : b) : (a ?? b)
  }

  return { v: 1, follows, lists, positions: Object.fromEntries(kept), sources, objects, removed }
}

const later = (a: SyncMark, b: SyncMark) => (a.sort_at - b.sort_at || a.id - b.id) > 0

/** JSON with keys in order, so equal copies give equal text. */
export function stableJson(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map(stableJson).join(',')}]`
  if (v && typeof v === 'object') {
    const o = v as Record<string, unknown>
    return `{${Object.keys(o)
      .filter((k) => o[k] !== undefined)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${stableJson(o[k])}`)
      .join(',')}}`
  }
  return JSON.stringify(v ?? null)
}

export const sameDoc = (a: SyncDoc, b: SyncDoc) => stableJson(a) === stableJson(b)

// The sync code: 15 random bytes as 24 letters and digits (Crockford's
// base32: no I, L, O or U to misread), shown in groups of four.
const ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'
export const CODE_BYTES = 15

export function encodeCode(bytes: Uint8Array): string {
  let out = ''
  let bits = 0
  let value = 0
  for (const b of bytes) {
    value = ((value << 8) | b) & 0xfff
    bits += 8
    while (bits >= 5) {
      out += ALPHABET[(value >>> (bits - 5)) & 31]
      bits -= 5
    }
  }
  if (bits > 0) out += ALPHABET[(value << (5 - bits)) & 31]
  return out.replace(/(.{4})(?=.)/g, '$1-')
}

/** A code as typed (any case, spaces, dashes, O for 0...) → its bytes, or null. */
export function decodeCode(text: string): Uint8Array | null {
  const clean = text
    .toUpperCase()
    .replace(/[^0-9A-Z]/g, '')
    .replace(/O/g, '0')
    .replace(/[IL]/g, '1')
  if (clean.length !== Math.ceil((CODE_BYTES * 8) / 5)) return null
  const out = new Uint8Array(CODE_BYTES)
  let bits = 0
  let value = 0
  let i = 0
  for (const ch of clean) {
    const d = ALPHABET.indexOf(ch)
    if (d < 0) return null
    value = ((value << 5) | d) & 0xfff
    bits += 5
    if (bits >= 8) {
      out[i++] = (value >>> (bits - 8)) & 255
      bits -= 8
    }
  }
  return out
}
