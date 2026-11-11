// 同步: the same follows, likes, saves, history, 刷到过, positions, channel
// bar, settings and own sources on every device paired by one sync code,
// kept up to date on their own. No account: the code (random, made here) is
// the only key.
//
// From the code come a token, by which the server (readers.rs, or the
// Worker) knows the copy, and an AES key it never sees: the copy is
// compressed and encrypted here. A device reads the copy when the site opens,
// when its tab comes back and every couple of minutes while it shows, merges
// it with its own (syncDoc.ts), and writes back what it has that the copy
// lacks, naming the version it read; if another device wrote in between, it
// reads and merges again. Its own changes go out a few seconds after they
// happen (opening and scrolling, a minute after).

import { ref, watch } from 'vue'
import { onChange } from './changes'
import { cleanPrefs, prefs } from './channels'
import { follows, restoreFollows } from './follow'
import { cleanEntries, entries, LIST_NAMES, restoreList, setSettings, settings, type Entry } from './library'
import { cleanOptions, options } from './options'
import { allPositions, cleanPositions, restorePositions } from './positions'
import { loadCapabilities } from './server'
import { cleanSources, ownSources, setOwnSources, type OwnSource } from './sources'
import { CODE_BYTES, decodeCode, encodeCode, mergeDocs, sameDoc, stableJson, type SyncDoc } from './syncDoc'

type Doc = SyncDoc<Entry, OwnSource>

/** This device's tie to a sync code. */
interface Link {
  code: string
  /** The server's version this device last matched; 0 = none yet. */
  version: number
  /** The last change here that a write carried, ms. */
  pushedAt: number
  lastSync: number
}

/** When things changed here, for merging (syncDoc.ts): kept even while sync is off. */
interface Marks {
  changedAt: number
  followedAt: Record<string, number>
  touched: Record<string, number>
  removed: Record<string, Record<string, number>>
}

const LINK_KEY = 'sync'
const MARKS_KEY = 'sync:marks'
const HARD_MS = 3000
const SOFT_MS = 60_000
const POLL_MS = 120_000
const MAX_BYTES = 1_900_000

function read<T>(key: string): T | null {
  try {
    return JSON.parse(localStorage.getItem(key) ?? 'null')
  } catch {
    return null
  }
}
function write(key: string, value: unknown) {
  try {
    if (value === null) localStorage.removeItem(key)
    else localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // Storage blocked: this visit only.
  }
}

const readLink = () => {
  const l = read<Link>(LINK_KEY)
  return l && typeof l.code === 'string' && decodeCode(l.code) ? l : null
}

function readMarks(): Marks {
  const m = read<Partial<Marks>>(MARKS_KEY) ?? {}
  return { changedAt: m.changedAt ?? 0, followedAt: m.followedAt ?? {}, touched: m.touched ?? {}, removed: m.removed ?? {} }
}

/** Marks only grow between syncs: the later of each, from every tab. */
function mergeMarks(a: Marks, b: Marks): Marks {
  const max = (x: Record<string, number>, y: Record<string, number>) => {
    const out = { ...x }
    for (const [k, v] of Object.entries(y)) if (!(out[k] >= v)) out[k] = v
    return out
  }
  const removed: Marks['removed'] = { ...a.removed }
  for (const [col, keys] of Object.entries(b.removed)) removed[col] = max(removed[col] ?? {}, keys)
  return {
    changedAt: Math.max(a.changedAt, b.changedAt),
    followedAt: max(a.followedAt, b.followedAt),
    touched: max(a.touched, b.touched),
    removed,
  }
}

let marks = readMarks()
let marksTimer: ReturnType<typeof setTimeout> | undefined
function saveMarks() {
  clearTimeout(marksTimer)
  marksTimer = undefined
  marks = mergeMarks(readMarks(), marks)
  write(MARKS_KEY, marks)
}

export interface SyncStatus {
  /** Paired with a code. */
  on: boolean
  busy: boolean
  error: string | null
  lastSync: number
  /** Turned off from another device. */
  gone: boolean
}

export const syncStatus = ref<SyncStatus>({ on: !!readLink(), busy: false, error: null, lastSync: readLink()?.lastSync ?? 0, gone: false })

/** A code from a pairing link (#sync=...), until the reader says yes or no. */
export const joinOffer = ref<string | null>(null)
{
  const m = /^#sync=([0-9A-Za-z-]+)$/.exec(location.hash)
  if (m) {
    // Out of the address bar (and its history) before anything else reads it.
    history.replaceState(history.state, '', location.pathname + location.search)
    if (decodeCode(m[1])) joinOffer.value = encodeCode(decodeCode(m[1])!)
  }
}

export const syncSupported = () => !!globalThis.crypto?.subtle && typeof CompressionStream !== 'undefined'

// ---- The copy ----

let applying = false

function makeDoc(): Doc {
  marks = mergeMarks(readMarks(), marks)
  return {
    v: 1,
    follows: Object.fromEntries(follows.value.map((id) => [String(id), marks.followedAt[id] ?? 0])),
    lists: Object.fromEntries(LIST_NAMES.map((n) => [n, entries(n).value.map((e) => ({ ...e }))])),
    positions: allPositions(),
    sources: Object.fromEntries(ownSources.value.map((s) => [s.url, { ...s }])),
    objects: {
      channels: { at: marks.touched.channels ?? 0, value: JSON.parse(JSON.stringify(prefs.value)) },
      options: { at: marks.touched.options ?? 0, value: JSON.parse(JSON.stringify(options.value)) },
      settings: { at: marks.touched.settings ?? 0, value: { ...settings.value } },
    },
    removed: marks.removed,
  }
}

const differs = (a: unknown, b: unknown) => stableJson(a) !== stableJson(b)

/** Puts a merged copy in place here; not a change of the reader's. */
function applyDoc(d: Doc) {
  applying = true
  try {
    const order = new Map(follows.value.map((id, i) => [id, i]))
    const ids = Object.entries(d.follows ?? {})
      .map(([id, at]) => [Number(id), at] as const)
      .filter(([id]) => Number.isInteger(id) && id > 0)
      .sort((a, b) => b[1] - a[1] || (order.get(a[0]) ?? 1e9) - (order.get(b[0]) ?? 1e9))
      .map(([id]) => id)
    if (differs(ids, follows.value)) restoreFollows(ids, 'replace')
    for (const n of LIST_NAMES) {
      const next = cleanEntries(d.lists?.[n] ?? [])
      if (differs(next, entries(n).value)) restoreList(n, next, 'replace')
    }
    const positions = cleanPositions(d.positions)
    if (differs(positions, allPositions())) restorePositions(positions)
    const sources = cleanSources(Object.values(d.sources ?? {}))
    if (differs(sources, ownSources.value)) setOwnSources(sources)
    const o = d.objects ?? {}
    if (o.channels && differs(o.channels.value, prefs.value)) prefs.value = cleanPrefs(o.channels.value)
    if (o.options && differs(o.options.value, options.value)) options.value = cleanOptions(o.options.value)
    const s = o.settings?.value as { history?: unknown; seen?: unknown } | undefined
    if (s && differs(s, settings.value)) setSettings({ history: s.history !== false, seen: s.seen !== false }, true)
    marks = {
      changedAt: marks.changedAt,
      followedAt: { ...d.follows },
      touched: { channels: o.channels?.at ?? 0, options: o.options?.at ?? 0, settings: o.settings?.at ?? 0 },
      removed: d.removed ?? {},
    }
    write(MARKS_KEY, marks)
  } finally {
    applying = false
  }
}

// ---- Keys, sealing ----

const enc = new TextEncoder()
const AAD = enc.encode('blogscroll sync 1')
const hex = (b: Uint8Array) => [...b].map((x) => x.toString(16).padStart(2, '0')).join('')

const keyCache = new Map<string, Promise<{ token: string; key: CryptoKey }>>()
function keysFor(code: string) {
  let k = keyCache.get(code)
  if (!k) {
    k = (async () => {
      const secret = decodeCode(code)!
      const base = await crypto.subtle.importKey('raw', secret as BufferSource, 'HKDF', false, ['deriveBits', 'deriveKey'])
      const params = (info: string) => ({ name: 'HKDF', hash: 'SHA-256', salt: enc.encode('blogscroll sync'), info: enc.encode(info) })
      const token = hex(new Uint8Array(await crypto.subtle.deriveBits(params('token'), base, 256)))
      const key = await crypto.subtle.deriveKey(params('key'), base, { name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt'])
      return { token, key }
    })()
    keyCache.set(code, k)
  }
  return k
}

async function pipe(bytes: Uint8Array, through: CompressionStream | DecompressionStream): Promise<Uint8Array<ArrayBuffer>> {
  return new Uint8Array(await new Response(new Blob([bytes as BlobPart]).stream().pipeThrough(through)).arrayBuffer())
}

/** [1][iv: 12][AES-GCM of the gzipped JSON] */
async function seal(doc: Doc, key: CryptoKey): Promise<Uint8Array> {
  const zipped = await pipe(enc.encode(JSON.stringify(doc)), new CompressionStream('gzip'))
  const iv = crypto.getRandomValues(new Uint8Array(12))
  const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: AAD }, key, zipped))
  const out = new Uint8Array(1 + iv.length + sealed.length)
  out.set([1])
  out.set(iv, 1)
  out.set(sealed, 1 + iv.length)
  return out
}

async function unseal(bytes: Uint8Array, key: CryptoKey): Promise<Doc> {
  if (bytes[0] !== 1) throw new Error('同步数据来自更新的版本，请先刷新页面')
  let zipped: ArrayBuffer
  try {
    zipped = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: bytes.slice(1, 13), additionalData: AAD }, key, bytes.slice(13))
  } catch {
    throw new Error('同步数据解不开，可能已损坏')
  }
  const doc = JSON.parse(new TextDecoder().decode(await pipe(new Uint8Array(zipped), new DecompressionStream('gzip'))))
  if (doc?.v !== 1) throw new Error('同步数据来自更新的版本，请先刷新页面')
  return doc
}

/** Within the size the server keeps: the long, less precious lists shorter if need be. */
async function sealFitting(doc: Doc, key: CryptoKey): Promise<Uint8Array> {
  for (const keep of [Infinity, 300, 50, 0]) {
    const lists: Doc['lists'] = { ...doc.lists, seen: doc.lists.seen?.slice(0, keep) ?? [] }
    if (keep === 0) lists.history = lists.history?.slice(0, 100) ?? []
    const d = { ...doc, lists }
    const sealed = await seal(d, key)
    if (sealed.length <= MAX_BYTES) return sealed
  }
  throw new Error('要同步的数据太多了')
}

// ---- Talking to the server ----

class Gone extends Error {}

const versionOf = (r: Response) => Number(r.headers.get('etag')?.replace(/\D/g, '')) || 0

async function withLock(run: () => Promise<void>) {
  // One tab at a time; they share this browser's storage.
  if (navigator.locks) await navigator.locks.request('blogscroll-sync', run)
  else await run()
}

let running = false
let again = false
let failures = 0
let timer: ReturnType<typeof setTimeout> | undefined
let timerAt = Infinity

function schedule(ms: number) {
  if (!readLink() || Date.now() + ms >= timerAt) return
  clearTimeout(timer)
  timerAt = Date.now() + ms
  timer = setTimeout(() => {
    timerAt = Infinity
    void syncNow()
  }, ms)
}

/** Reads the copy, merges, and writes back what changed. Safe to call any time. */
export async function syncNow(): Promise<void> {
  if (!readLink() || !syncSupported()) return
  if (running) {
    again = true
    return
  }
  running = true
  syncStatus.value = { ...syncStatus.value, busy: true }
  try {
    if (!(await loadCapabilities()).sync) throw new Error('这里的服务器不提供同步')
    await withLock(once)
    failures = 0
    syncStatus.value = { ...syncStatus.value, error: null, lastSync: readLink()?.lastSync ?? 0 }
  } catch (e) {
    if (e instanceof Gone) {
      write(LINK_KEY, null)
      syncStatus.value = { on: false, busy: false, error: null, lastSync: 0, gone: true }
      return
    }
    failures++
    syncStatus.value = { ...syncStatus.value, error: (e as Error).message || '同步失败' }
    schedule(Math.min(10 * 60_000, 15_000 * 2 ** failures))
  } finally {
    running = false
    syncStatus.value = { ...syncStatus.value, busy: false, on: !!readLink() }
    if (again) {
      again = false
      schedule(500)
    }
  }
}

async function once() {
  for (let attempt = 0; attempt < 4; attempt++) {
    // Another tab may have synced meanwhile.
    const link = readLink()
    if (!link) return
    const { token, key } = await keysFor(link.code)
    const auth = { authorization: `Bearer ${token}` }
    let res: Response
    try {
      res = await fetch('/api/sync', {
        headers: link.version ? { ...auth, 'if-none-match': `"${link.version}"` } : auth,
        cache: 'no-store',
      })
    } catch {
      throw new Error('连不上服务器，稍后自动重试')
    }
    if (res.status === 410) throw new Gone()
    if (res.status !== 200 && res.status !== 304 && res.status !== 404) throw new Error(`服务器出错（HTTP ${res.status}）`)

    let base = res.status === 404 ? 0 : link.version
    let local = makeDoc()
    const changedAt = marks.changedAt
    let needed = changedAt > link.pushedAt || res.status === 404
    if (res.status === 200) {
      base = versionOf(res)
      const remote = await unseal(new Uint8Array(await res.arrayBuffer()), key)
      applyDoc(mergeDocs(local, remote))
      local = makeDoc()
      needed = !sameDoc(local, remote)
    }
    if (!needed) {
      write(LINK_KEY, { ...link, version: base, pushedAt: Math.max(link.pushedAt, changedAt), lastSync: Date.now() })
      return
    }
    const put = await fetch('/api/sync', {
      method: 'PUT',
      headers: {
        ...auth,
        'content-type': 'application/octet-stream',
        ...(base ? { 'if-match': `"${base}"` } : { 'if-none-match': '*' }),
      },
      body: (await sealFitting(local, key)) as BodyInit,
    })
    if (put.status === 412) continue
    if (put.status === 410) throw new Gone()
    if (put.status === 413) throw new Error('要同步的数据太多了')
    if (!put.ok) throw new Error(`服务器出错（HTTP ${put.status}）`)
    write(LINK_KEY, { ...link, version: versionOf(put), pushedAt: changedAt, lastSync: Date.now() })
    // Changed while writing: once more, soon.
    if (readMarks().changedAt > changedAt) schedule(HARD_MS)
    return
  }
  throw new Error('别的设备正在同步，稍后自动重试')
}

// ---- What the reader does ----

export function newSyncCode(): string {
  return encodeCode(crypto.getRandomValues(new Uint8Array(CODE_BYTES)))
}

/** Starts syncing with `code`: a new one, or one from another device (what is here is merged in). */
export async function startSync(code: string) {
  const bytes = decodeCode(code)
  if (!bytes) throw new Error('同步码不对：应该是 24 个字母和数字')
  write(LINK_KEY, { code: encodeCode(bytes), version: 0, pushedAt: 0, lastSync: 0 } satisfies Link)
  joinOffer.value = null
  syncStatus.value = { on: true, busy: false, error: null, lastSync: 0, gone: false }
  await syncNow()
}

/** This device stops; the others and the copy on the server stay. */
export function stopSync() {
  write(LINK_KEY, null)
  syncStatus.value = { on: false, busy: false, error: null, lastSync: 0, gone: false }
}

/** Removes the copy from the server: every device paired with the code stops. */
export async function deleteSync() {
  const link = readLink()
  if (!link) return
  const { token } = await keysFor(link.code)
  const res = await fetch('/api/sync', { method: 'DELETE', headers: { authorization: `Bearer ${token}` } })
  if (!res.ok) throw new Error(`没能删除（HTTP ${res.status}）`)
  stopSync()
}

export const syncCode = () => readLink()?.code ?? null

/** Opens on another device and offers to pair it (the code stays out of server logs: after #). */
export function pairingLink(code: string): string {
  return `${location.origin}${import.meta.env.BASE_URL}settings#sync=${code.replace(/-/g, '')}`
}

// ---- Keeping up ----

onChange((c) => {
  if (applying) return
  const now = Date.now()
  if (c.kind === 'removed') {
    const col = (marks.removed[c.collection] ??= {})
    for (const k of c.keys) col[String(k)] = now
  } else if (c.kind === 'followed') {
    marks.followedAt[c.id] = now
  }
  marks.changedAt = now
  marksTimer ??= setTimeout(saveMarks, 300)
  if (c.kind === 'changed') schedule(c.soft ? SOFT_MS : HARD_MS)
})

for (const [name, value] of [
  ['channels', () => prefs.value],
  ['options', () => options.value],
  ['settings', () => settings.value],
] as const) {
  watch(
    value,
    () => {
      if (applying) return
      marks.touched[name] = marks.changedAt = Date.now()
      marksTimer ??= setTimeout(saveMarks, 300)
      schedule(HARD_MS)
    },
    { deep: true, flush: 'sync' },
  )
}

let lastPull = 0
function pull() {
  if (!readLink() || document.visibilityState !== 'visible') return
  lastPull = Date.now()
  void syncNow()
}

document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible') {
    if (Date.now() - lastPull > 15_000) pull()
  } else if (readLink() && marks.changedAt > (readLink()?.pushedAt ?? 0)) {
    // Leaving: what changed goes now.
    saveMarks()
    void syncNow()
  }
})
window.addEventListener('online', pull)
setInterval(pull, POLL_MS)
if (readLink()) setTimeout(pull, 1000)

// Whether this server can sync at all (shown in settings).
void loadCapabilities()
