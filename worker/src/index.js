// The static site on Cloudflare. The files the publish workflow builds
// (frontend/dist: the app, the exported data, the pages for search engines)
// are served as they are; this answers /api/*, what plain static hosting
// cannot, the same as the server does (backend/src/readers.rs):
//
//   /api/capabilities   that the two below are here
//   /api/sync           a reader's copy of their data, encrypted in their
//                       browser, by version (frontend/src/sync.ts); in D1
//   /api/fetch?url=     a feed a reader added, fetched for their browser
//                       (frontend/src/sources.ts); kept a quarter of an hour,
//                       so readers of the same feed share one fetch
//
// Old sync copies nobody wrote for 400 days are dropped once a day.

const MAX_SYNC_BYTES = 1_900_000
const MAX_SYNC_DOCS = 20_000
const MAX_FEED_BYTES = 4 * 1024 * 1024
const FEED_TTL = 15 * 60
const FAILURE_TTL = 10 * 60
const KEEP_DAYS = 400
const ACCEPT_FEED =
  'application/rss+xml, application/atom+xml, application/feed+json, application/xml;q=0.9, text/xml;q=0.9, */*;q=0.5'

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url)
    try {
      if (url.pathname === '/api/capabilities') return json({ sync: true, fetch: true })
      if (url.pathname === '/api/sync') return await sync(request, env)
      if (url.pathname === '/api/fetch' && request.method === 'GET') return await fetchFeed(url, env, ctx)
    } catch (e) {
      console.error(e)
      return error(500, 'server error')
    }
    if (url.pathname.startsWith('/api/')) return error(404, 'not found')
    return env.ASSETS.fetch(request)
  },

  async scheduled(_event, env) {
    await ready(env)
    const before = Math.floor(Date.now() / 1000) - KEEP_DAYS * 86400
    await env.DB.prepare('DELETE FROM sync_docs WHERE updated_at < ?').bind(before).run()
  },
}

function json(value, status = 200, headers = {}) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store', ...headers },
  })
}

const error = (status, message, headers) => json({ error: message }, status, headers)

// ---- Sync ----

// The table, made on first use (the database itself is made by the first deploy).
let schema
function ready(env) {
  schema ??= env.DB.prepare(
    `CREATE TABLE IF NOT EXISTS sync_docs (
      id TEXT PRIMARY KEY,
      version INTEGER NOT NULL,
      data BLOB NOT NULL,
      deleted INTEGER NOT NULL DEFAULT 0,
      updated_at INTEGER NOT NULL
    )`,
  )
    .run()
    .catch((e) => {
      schema = undefined
      throw e
    })
  return schema
}

/** `Bearer <64 hex>` → its SHA-256: what is stored is not enough to write. */
async function docId(request) {
  const token = /^Bearer ([0-9a-f]{64})$/i.exec(request.headers.get('authorization') ?? '')?.[1]?.toLowerCase()
  if (!token) return null
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(token))
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

/** `"12"` → 12 */
const versionOf = (v) => (v && /^(W\/)?"?\d+"?$/.test(v.trim()) ? Number(v.replace(/\D/g, '')) : null)
const etag = (v) => ({ etag: `"${v}"` })
const bytesOf = (data) => (data instanceof ArrayBuffer ? new Uint8Array(data) : Uint8Array.from(data ?? []))

async function sync(request, env) {
  const id = await docId(request)
  if (!id) return error(401, 'missing or malformed token')
  await ready(env)
  const db = env.DB
  const now = Math.floor(Date.now() / 1000)
  const gone = () => error(410, 'sync was turned off for this code')

  if (request.method === 'GET') {
    const row = await db.prepare('SELECT version, data, deleted FROM sync_docs WHERE id = ?').bind(id).first()
    if (!row) return error(404, 'nothing synced with this code yet')
    if (row.deleted) return gone()
    if (versionOf(request.headers.get('if-none-match')) === row.version) {
      return new Response(null, { status: 304, headers: { ...etag(row.version), 'cache-control': 'no-store' } })
    }
    return new Response(bytesOf(row.data), {
      headers: { ...etag(row.version), 'content-type': 'application/octet-stream', 'cache-control': 'no-store' },
    })
  }

  if (request.method === 'PUT') {
    if (Number(request.headers.get('content-length')) > MAX_SYNC_BYTES) return error(413, 'too large')
    const body = new Uint8Array(await request.arrayBuffer())
    if (body.length > MAX_SYNC_BYTES) return error(413, 'too large')
    if (!body.length) return error(400, 'empty body')
    let written = null
    if (request.headers.get('if-none-match') === '*') {
      const { n } = await db.prepare('SELECT COUNT(*) AS n FROM sync_docs').first()
      if (n >= MAX_SYNC_DOCS) return error(507, 'no room for another sync code')
      const r = await db
        .prepare(
          'INSERT INTO sync_docs (id, version, data, deleted, updated_at) VALUES (?, 1, ?, 0, ?) ON CONFLICT (id) DO NOTHING',
        )
        .bind(id, body, now)
        .run()
      if (r.meta.changes === 1) written = 1
    } else {
      const base = versionOf(request.headers.get('if-match'))
      if (base === null) return error(428, 'send If-Match, or If-None-Match: * for the first write')
      const r = await db
        .prepare(
          'UPDATE sync_docs SET version = version + 1, data = ?, updated_at = ? WHERE id = ? AND version = ? AND deleted = 0',
        )
        .bind(body, now, id, base)
        .run()
      if (r.meta.changes === 1) written = base + 1
    }
    if (written !== null) return json({ version: written }, 200, etag(written))
    // Someone wrote first, or sync was turned off.
    const row = await db.prepare('SELECT deleted FROM sync_docs WHERE id = ?').bind(id).first()
    return row?.deleted ? gone() : error(412, 'changed since; read it again')
  }

  if (request.method === 'DELETE') {
    await db
      .prepare(
        `INSERT INTO sync_docs (id, version, data, deleted, updated_at) VALUES (?, 1, x'', 1, ?)
         ON CONFLICT (id) DO UPDATE SET version = version + 1, data = x'', deleted = 1, updated_at = excluded.updated_at`,
      )
      .bind(id, now)
      .run()
    return new Response(null, { status: 204, headers: { 'cache-control': 'no-store' } })
  }

  return error(405, 'method not allowed')
}

// ---- Feeds readers add themselves ----

/** The open internet only: no names or addresses of a private network. */
function allowed(url) {
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return false
  const host = url.hostname.toLowerCase().replace(/\.$/, '')
  if (host.startsWith('[')) {
    const v6 = host.slice(1, -1)
    if (/^::ffff:/.test(v6)) return publicV4(v6.slice(7))
    return !(v6 === '::' || v6 === '::1' || /^f[cd]/.test(v6) || /^fe[89ab]/.test(v6) || /^ff/.test(v6) || /^2001:db8:/.test(v6) || /^64:ff9b:/.test(v6))
  }
  if (/^\d+\.\d+\.\d+\.\d+$/.test(host)) return publicV4(host)
  return (
    host.includes('.') &&
    host !== 'localhost' &&
    !['.localhost', '.local', '.internal', '.lan', '.home.arpa'].some((s) => host.endsWith(s))
  )
}

function publicV4(ip) {
  const [a, b, c] = ip.split('.').map(Number)
  if ([a, b, c].some((n) => !(n >= 0 && n <= 255))) return false
  return !(
    a === 0 ||
    a === 10 ||
    a === 127 ||
    a >= 224 ||
    (a === 100 && b >= 64 && b < 128) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b < 32) ||
    (a === 192 && b === 168) ||
    (a === 192 && b === 0 && (c === 0 || c === 2)) ||
    (a === 198 && (b === 18 || b === 19 || (b === 51 && c === 100))) ||
    (a === 203 && b === 0 && c === 113)
  )
}

/** What a body is, by its start: a feed or an OPML list, a web page, or neither. */
function sniff(bytes) {
  const head = new TextDecoder().decode(bytes.subarray(0, 4096)).toLowerCase().replace(/^\uFEFF/, '').trimStart()
  if (head.startsWith('{')) return head.includes('jsonfeed.org') ? 'feed' : 'other'
  if (head.startsWith('<!doctype html') || head.startsWith('<html')) return 'page'
  const xml = head.startsWith('<')
  if (xml && ['<rss', '<feed', '<rdf:rdf', '<opml'].some((t) => head.includes(t))) return 'feed'
  return xml && head.includes('<html') ? 'page' : 'other'
}

/** Feeds a page links to, best first (as html.rs `feed_links`). */
async function feedLinks(bytes, base) {
  const found = []
  const ranks = {
    'application/atom+xml': 0,
    'application/rss+xml': 0,
    'application/feed+json': 1,
    'application/json': 1,
    'application/rdf+xml': 2,
    'application/xml': 2,
    'text/xml': 2,
  }
  await new HTMLRewriter()
    .on('link[rel][href]', {
      element(el) {
        const rel = (el.getAttribute('rel') ?? '').toLowerCase().split(/\s+/)
        const rank = ranks[(el.getAttribute('type') ?? '').toLowerCase().trim()]
        const title = (el.getAttribute('title') ?? '').toLowerCase()
        const href = el.getAttribute('href') ?? ''
        if (!rel.includes('alternate') || rank === undefined) return
        if (title.includes('comment') || title.includes('评论') || href.includes('comments')) return
        try {
          const u = new URL(href, base)
          if (/^https?:$/.test(u.protocol) && !found.some((f) => f.url === u.href)) found.push({ rank, url: u.href })
        } catch {
          // Not an address.
        }
      },
    })
    .transform(new Response(bytes))
    .arrayBuffer()
  return found.sort((a, b) => a.rank - b.rank).map((f) => f.url)
}

async function readCapped(res) {
  const reader = res.body.getReader()
  const parts = []
  let size = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    size += value.length
    if (size > MAX_FEED_BYTES) {
      await reader.cancel()
      return null
    }
    parts.push(value)
  }
  const out = new Uint8Array(size)
  let at = 0
  for (const p of parts) {
    out.set(p, at)
    at += p.length
  }
  return out
}

/** Fetches it, following up to 5 redirects that stay on the open internet. */
async function answer(target, env) {
  const failed = (status, message, retryAfter) => ({ status, message, retryAfter })
  let url = target
  let res
  for (let hop = 0; ; hop++) {
    if (!allowed(url)) return failed(403, '不能访问内网或本机的地址')
    try {
      res = await fetch(url, {
        headers: { 'user-agent': env.USER_AGENT || 'blogScroll (RSS reader)', accept: ACCEPT_FEED },
        redirect: 'manual',
        signal: AbortSignal.timeout(15_000),
      })
    } catch (e) {
      return e?.name === 'TimeoutError' ? failed(504, '对方网站没有及时响应') : failed(502, `连不上对方网站（${e?.message ?? e}）`)
    }
    const next = res.status >= 300 && res.status < 400 && res.headers.get('location')
    if (!next) break
    if (hop >= 5) return failed(502, '对方网站转了太多次')
    url = new URL(next, url)
  }
  if (res.status === 429 || res.status === 503) {
    const wait = Number(res.headers.get('retry-after'))
    return failed(503, '对方网站暂时不让访问，稍后再试', wait > 0 ? wait : undefined)
  }
  if (!res.ok) return failed(502, `对方网站返回 HTTP ${res.status}`)
  const body = await readCapped(res)
  if (!body) return failed(502, '内容太大，不像是 RSS')
  const kind = sniff(body)
  if (kind === 'feed') return { body, type: res.headers.get('content-type') ?? '', url: url.href }
  if (kind === 'page') return { feeds: await feedLinks(body, url), url: url.href }
  return failed(422, '不是 RSS、Atom 或 OPML')
}

/** As the reader gets it (`private`), or as kept for everyone (`public`). */
function respond(a, cache) {
  const headers = { 'cache-control': `${cache}, max-age=${a.message ? FAILURE_TTL : FEED_TTL}`, 'x-content-type-options': 'nosniff' }
  if (a.message) {
    const wait = a.retryAfter && Math.min(6 * 3600, Math.max(60, a.retryAfter))
    if (wait) Object.assign(headers, { 'retry-after': String(a.retryAfter), 'cache-control': `${cache}, max-age=${wait}` })
    return new Response(JSON.stringify({ error: a.message }), { status: a.status, headers: { ...headers, 'content-type': 'application/json' } })
  }
  if (a.feeds) {
    return new Response(JSON.stringify({ feeds: a.feeds, final_url: a.url }), {
      headers: { ...headers, 'content-type': 'application/json', 'x-kind': 'page' },
    })
  }
  // A download, never a page of this site: what a feed holds must not run here.
  return new Response(a.body, {
    headers: {
      ...headers,
      'content-type': 'application/octet-stream',
      'content-disposition': 'attachment',
      'content-security-policy': "sandbox; default-src 'none'",
      'x-kind': 'feed',
      'x-feed-type': a.type,
      'x-final-url': a.url,
    },
  })
}

async function fetchFeed(url, env, ctx) {
  let target
  try {
    const raw = (url.searchParams.get('url') ?? '').trim()
    target = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `https://${raw}`)
  } catch {
    return error(400, '地址无效')
  }
  // One fetch per address per quarter hour, for everyone.
  const key = new Request(new URL(`/api/fetch-cache/${encodeURIComponent(target.href)}`, url))
  const cached = await caches.default.match(key)
  if (cached) {
    const r = new Response(cached.body, cached)
    r.headers.set('cache-control', r.headers.get('cache-control').replace('public', 'private'))
    return r
  }
  const a = await answer(target, env)
  ctx.waitUntil(caches.default.put(key, respond(a, 'public')))
  return respond(a, 'private')
}
