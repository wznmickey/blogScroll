// node --experimental-strip-types --test tests/ (npm test)
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { decodeCode, encodeCode, mergeDocs, sameDoc, stableJson, type SyncDoc } from '../src/syncDoc.ts'

const NOW = 1_800_000_000_000
const DAY = 86_400_000

function doc(patch: Partial<SyncDoc> = {}): SyncDoc {
  return { v: 1, follows: {}, lists: {}, positions: {}, sources: {}, objects: {}, removed: {}, ...patch }
}
const entry = (id: number, at: number, count?: number) => ({ at, post: { id }, ...(count ? { count } : {}) })

test('keeps what either device has', () => {
  const a = doc({ follows: { 1: 0, 2: 100 }, lists: { likes: [entry(10, 5)] } })
  const b = doc({ follows: { 3: 50 }, lists: { likes: [entry(11, 7)], saves: [entry(12, 1)] } })
  const m = mergeDocs(a, b, NOW)
  assert.deepEqual(m.follows, { 1: 0, 2: 100, 3: 50 })
  assert.deepEqual(
    m.lists.likes.map((e) => e.post.id),
    [11, 10],
  )
  assert.deepEqual(
    m.lists.saves.map((e) => e.post.id),
    [12],
  )
})

test('a removal wins over an older add, a newer add over the removal', () => {
  const a = doc({ follows: { 1: 0, 2: 0 }, lists: { likes: [entry(10, 100), entry(11, 100)] } })
  // The other device unfollowed 1 and unliked 10 and 11 at 200, then liked 11 again at 300.
  const b = doc({
    lists: { likes: [entry(11, 300)] },
    removed: { follows: { 1: NOW - DAY }, likes: { 10: 200, 11: 200 } },
  })
  const m = mergeDocs(a, b, NOW)
  assert.deepEqual(m.follows, { 2: 0 })
  assert.deepEqual(
    m.lists.likes.map((e) => [e.post.id, e.at]),
    [[11, 300]],
  )
  // Both ways round.
  assert.equal(stableJson(mergeDocs(b, a, NOW).lists), stableJson(m.lists))
  // Removal marks are kept for a while, then dropped.
  assert.deepEqual(m.removed.follows, { 1: NOW - DAY })
  assert.equal(mergeDocs(a, b, NOW + 200 * DAY).removed.follows, undefined)
})

test('the newer copy of an entry, and the higher count for 刷到过', () => {
  const a = doc({ lists: { history: [entry(1, 10)], seen: [entry(5, 10, 4)] } })
  const b = doc({ lists: { history: [entry(1, 20)], seen: [entry(5, 30, 2)] } })
  const m = mergeDocs(a, b, NOW)
  assert.equal(m.lists.history[0].at, 20)
  assert.deepEqual(m.lists.seen[0], { at: 30, post: { id: 5 }, count: 4 })
})

test('settings: the newer one; on a tie the copy already synced', () => {
  const a = doc({ objects: { options: { at: 5, value: 'mine' }, channels: { at: 0, value: 'mine' } } })
  const b = doc({ objects: { options: { at: 3, value: 'theirs' }, channels: { at: 0, value: 'theirs' } } })
  const m = mergeDocs(a, b, NOW)
  assert.equal(m.objects.options.value, 'mine')
  assert.equal(m.objects.channels.value, 'theirs')
})

test('positions widen to what either has seen', () => {
  const p = (newest: number, oldest: number, at: number) => ({
    newest: { sort_at: newest, id: 1 },
    oldest: { sort_at: oldest, id: 1, title: String(oldest) },
    at,
  })
  const m = mergeDocs(doc({ positions: { latest: p(50, 20, 1) } }), doc({ positions: { latest: p(60, 30, 2) } }), NOW)
  assert.deepEqual(m.positions.latest, p(60, 20, 2))
})

test('sources: the newer one, unless removed later', () => {
  const s = (url: string, at: number, title = url) => ({ url, at, title })
  const a = doc({ sources: { x: s('x', 10, 'old'), y: s('y', 10) } })
  const b = doc({ sources: { x: s('x', 20, 'new') }, removed: { sources: { y: 15 } } })
  const m = mergeDocs(a, b, NOW)
  assert.deepEqual(Object.keys(m.sources), ['x'])
  assert.equal((m.sources.x as { title: string }).title, 'new')
})

test('equal copies compare equal whatever the key order', () => {
  const a = doc({ follows: { 1: 0, 2: 0 } })
  const b = { removed: {}, objects: {}, sources: {}, positions: {}, lists: {}, follows: { 2: 0, 1: 0 }, v: 1 } as SyncDoc
  assert.ok(sameDoc(a, b))
  assert.ok(!sameDoc(a, doc()))
})

test('sync codes survive being read back, mistyped letters included', () => {
  const bytes = Uint8Array.from({ length: 15 }, (_, i) => (i * 37 + 11) & 255)
  const code = encodeCode(bytes)
  assert.match(code, /^([0-9A-HJKMNP-TV-Z]{4}-){5}[0-9A-HJKMNP-TV-Z]{4}$/)
  assert.deepEqual(decodeCode(code), bytes)
  assert.deepEqual(decodeCode(code.toLowerCase().replace(/-/g, ' ')), bytes)
  assert.deepEqual(decodeCode(code.replace(/0/g, 'O').replace(/1/g, 'l')), bytes)
  assert.equal(decodeCode(code.slice(0, -1)), null)
  assert.equal(decodeCode(`${code.slice(0, -1)}U`), null)
  assert.deepEqual(decodeCode(encodeCode(new Uint8Array(15).fill(255))), new Uint8Array(15).fill(255))
})
