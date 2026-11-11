// Backup and restore of everything this reader keeps in the browser: followed
// blogs, likes, saves, history, 刷到过, where they got to in each feed, the
// channel bar and the settings page. A JSON file, so it can
// move to another browser or device, or survive clearing site data.

import { prefs, cleanPrefs } from './channels'
import { cleanOptions, options, type Options } from './options'
import { follows, restoreFollows } from './follow'
import { cleanEntries, entries, LIST_NAMES, restoreList, setSettings, settings, type Entry } from './library'
import { allPositions, cleanPositions, restorePositions, type Position } from './positions'
import { report } from './changes'
import { cleanSources, ownSources, setOwnSources, type OwnSource } from './sources'

const FORMAT = 'blogscroll-backup'
const VERSION = 1

export interface Backup {
  format: typeof FORMAT
  version: number
  exported_at: string
  follows: number[]
  likes: Entry[]
  saves: Entry[]
  history: Entry[]
  seen: Entry[]
  positions: Record<string, Position>
  channels: ReturnType<typeof cleanPrefs>
  settings: { history: boolean; seen: boolean }
  /** The settings page (weights, blocks, ...); missing in older backups. */
  options: Options | null
  /** Feeds and lists the reader added (sources.ts); missing in older backups. */
  sources: OwnSource[]
}

export function makeBackup(): Backup {
  return {
    format: FORMAT,
    version: VERSION,
    exported_at: new Date().toISOString(),
    follows: [...follows.value],
    likes: [...entries('likes').value],
    saves: [...entries('saves').value],
    history: [...entries('history').value],
    seen: [...entries('seen').value],
    positions: allPositions(),
    channels: JSON.parse(JSON.stringify(prefs.value)),
    settings: { ...settings.value },
    options: JSON.parse(JSON.stringify(options.value)),
    sources: JSON.parse(JSON.stringify(ownSources.value)),
  }
}

export function downloadBackup() {
  const blob = new Blob([JSON.stringify(makeBackup(), null, 1)], { type: 'application/json' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = `blogscroll-backup-${new Date().toISOString().slice(0, 10)}.json`
  document.body.append(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(a.href), 1000)
}

/** A backup read from a file, checked and cleaned; throws with a readable message. */
export function parseBackup(text: string): Backup {
  let v: Record<string, unknown>
  try {
    v = JSON.parse(text)
  } catch {
    throw new Error('不是有效的 JSON 文件')
  }
  if (v?.format !== FORMAT) throw new Error('这不是本站导出的备份文件')
  if (typeof v.version !== 'number' || v.version > VERSION) throw new Error('备份来自更新的版本，请先刷新页面')
  return {
    format: FORMAT,
    version: v.version,
    exported_at: typeof v.exported_at === 'string' ? v.exported_at : '',
    follows: Array.isArray(v.follows) ? v.follows.filter((n): n is number => Number.isInteger(n) && n > 0) : [],
    likes: cleanEntries(v.likes),
    saves: cleanEntries(v.saves),
    history: cleanEntries(v.history),
    // Backups from before 刷到过 existed have none.
    seen: cleanEntries(v.seen),
    positions: cleanPositions(v.positions),
    channels: cleanPrefs(v.channels),
    settings: {
      history: (v.settings as { history?: unknown } | undefined)?.history !== false,
      seen: (v.settings as { seen?: unknown } | undefined)?.seen !== false,
    },
    options: v.options ? cleanOptions(v.options) : null,
    sources: cleanSources(v.sources),
  }
}

/**
 * Merge: adds the backup to what is here (channel settings stay as they are).
 * Replace: makes this browser match the backup.
 */
export function restoreBackup(b: Backup, mode: 'merge' | 'replace') {
  // Replacing removes what the backup lacks, on synced devices too.
  if (mode === 'replace') {
    const lacks = (col: string, here: (string | number)[], kept: (string | number)[]) => {
      const keys = here.filter((k) => !kept.includes(k))
      if (keys.length) report({ kind: 'removed', collection: col, keys })
    }
    lacks('follows', [...follows.value], b.follows)
    for (const name of LIST_NAMES) lacks(name, entries(name).value.map((e) => e.post.id), b[name].map((e) => e.post.id))
    lacks('sources', ownSources.value.map((s) => s.url), b.sources.map((s) => s.url))
  }
  restoreFollows(b.follows, mode)
  for (const name of LIST_NAMES) restoreList(name, b[name], mode)
  if (mode === 'replace') {
    prefs.value = b.channels
    setSettings(b.settings)
    if (b.options) options.value = b.options
    restorePositions(b.positions)
    setOwnSources(b.sources)
  } else {
    restorePositions({ ...b.positions, ...allPositions() })
    const urls = new Set(ownSources.value.map((s) => s.url))
    setOwnSources([...ownSources.value, ...b.sources.filter((s) => !urls.has(s.url))])
  }
  report({ kind: 'changed' })
}
