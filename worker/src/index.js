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
//   /api/readers/feeds  the feeds readers added, registered (not who added
//                       them) for the crawler, which takes them in like the
//                       site's blogs; the crawl workflow reads the list (D1)
//   /api/refresh        a visitor asks for one blog to be read again
//
// The crawling itself is the crawl workflow's (GitHub Actions, the Rust
// crawler): every CRAWL_EVERY_HOURS this Worker's timer starts it, so data
// keeps coming whatever happens in the code repository; asked-for refreshes
// (and readers' new feeds) start a short run for just those blogs. Both need
// the secret GITHUB_TOKEN (a fine-grained token, Actions: read and write, on
// the code repository GITHUB_REPO). Once a day old entries are dropped: sync
// copies nobody wrote for 400 days, feeds nobody asked for in 90.

const MAX_SYNC_BYTES = 1_900_000
const MAX_SYNC_DOCS = 20_000
const MAX_OWN_FEEDS = 500
const MAX_READER_FEEDS = 5000
const READER_DAYS = 90
/** A blog is read again on request at most this often. */
const REFRESH_SECS = 30 * 60
/** Runs for refreshes start at most this often, at most this many an hour, for this many blogs. */
const REFRESH_GAP = 3 * 60
const REFRESHES_PER_HOUR = 12
const SITES_PER_RUN = 30
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
      if (url.pathname === '/api/capabilities') {
        return json({ sync: true, fetch: true, readers: true, refresh: !!env.GITHUB_TOKEN })
      }
      if (url.pathname === '/api/sync') return await sync(request, env)
      if (url.pathname === '/api/fetch' && request.method === 'GET') return await fetchFeed(url, env, ctx)
      if (url.pathname === '/api/readers/feeds' && request.method === 'POST') return await register(request, env, ctx)
      if (url.pathname === '/api/refresh' && request.method === 'POST') return await refresh(request, env, ctx)
    } catch (e) {
      console.error(e)
      return error(500, 'server error')
    }
    if (url.pathname.startsWith('/api/')) return error(404, 'not found')
    return env.ASSETS.fetch(request)
  },

  // Every five minutes (wrangler.jsonc).
  async scheduled(_event, env) {
    await ready(env)
    const now = Math.floor(Date.now() / 1000)
    if (now - (await getState(env, 'pruned')) >= 86400) {
      await env.DB.batch([
        env.DB.prepare('DELETE FROM sync_docs WHERE updated_at < ?').bind(now - KEEP_DAYS * 86400),
        env.DB.prepare('DELETE FROM reader_feeds WHERE last_at < ?').bind(now - READER_DAYS * 86400),
        env.DB.prepare('DELETE FROM seen_feeds WHERE at < ?').bind(now - 30 * 86400),
        env.DB.prepare('DELETE FROM refreshes WHERE requested_at < ?').bind(now - 86400),
      ])
      await setState(env, 'pruned', now)
    }
    if (!env.GITHUB_TOKEN) return
    await dispatchRefreshes(env)
    const every = Number(env.CRAWL_EVERY_HOURS ?? 3) * 3600
    if (every > 0 && now - (await getState(env, 'crawled')) >= every) {
      // Counted as started even if GitHub refused, so a broken token is not retried every five minutes.
      await setState(env, 'crawled', now)
      await dispatch(env, { crawl: 'true', scheduled: 'true' })
    }
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

// The tables, made on first use (the database itself is made by the first deploy).
let schema
function ready(env) {
  schema ??= env.DB.batch(
    [
      `CREATE TABLE IF NOT EXISTS sync_docs (
        id TEXT PRIMARY KEY, version INTEGER NOT NULL, data BLOB NOT NULL,
        deleted INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL)`,
      // Readers' own feeds, for the crawler: no reader, only when last asked for.
      'CREATE TABLE IF NOT EXISTS reader_feeds (url TEXT PRIMARY KEY, first_at INTEGER NOT NULL, last_at INTEGER NOT NULL)',
      // Feeds /api/fetch found to be feeds: only those may be registered.
      'CREATE TABLE IF NOT EXISTS seen_feeds (url TEXT PRIMARY KEY, at INTEGER NOT NULL)',
      // Blogs asked to be read again (`site`: an address or a reader's feed).
      'CREATE TABLE IF NOT EXISTS refreshes (site TEXT PRIMARY KEY, requested_at INTEGER NOT NULL, dispatched_at INTEGER)',
      'CREATE TABLE IF NOT EXISTS state (key TEXT PRIMARY KEY, value INTEGER NOT NULL)',
    ].map((sql) => env.DB.prepare(sql)),
  ).catch((e) => {
    schema = undefined
    throw e
  })
  return schema
}

async function getState(env, key) {
  return (await env.DB.prepare('SELECT value FROM state WHERE key = ?').bind(key).first('value')) ?? 0
}
function setState(env, key, value) {
  return env.DB.prepare('INSERT INTO state (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value')
    .bind(key, value)
    .run()
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
  // A real feed now: it may be registered for crawling.
  if (a.body) ctx.waitUntil(seen(env, target.href))
  return respond(a, 'private')
}

async function seen(env, url) {
  await ready(env)
  await env.DB.prepare('INSERT INTO seen_feeds (url, at) VALUES (?, ?) ON CONFLICT (url) DO UPDATE SET at = excluded.at')
    .bind(url, Math.floor(Date.now() / 1000))
    .run()
}

// ---- Readers' own feeds, and reading a blog again ----

/** All the feeds a reader has: the new ones are crawled from the next run (soon). */
async function register(request, env, ctx) {
  const body = await request.json().catch(() => null)
  const feeds = Array.isArray(body?.feeds)
    ? [...new Set(body.feeds.filter((f) => typeof f === 'string' && f.length <= 2000).map((f) => f.trim()))].slice(0, MAX_OWN_FEEDS)
    : []
  await ready(env)
  const now = Math.floor(Date.now() / 1000)
  let count = await env.DB.prepare('SELECT COUNT(*) AS n FROM reader_feeds').first('n')
  const fresh = []
  for (const url of feeds) {
    let u
    try {
      u = new URL(url)
    } catch {
      continue
    }
    if (!allowed(u)) continue
    const known = await env.DB.prepare('UPDATE reader_feeds SET last_at = ? WHERE url = ?').bind(now, url).run()
    if (known.meta.changes) continue
    // New: only a feed this Worker has fetched as one (not any address), and room for it.
    if (count >= MAX_READER_FEEDS || !(await env.DB.prepare('SELECT 1 AS y FROM seen_feeds WHERE url = ?').bind(url).first('y'))) continue
    await env.DB.prepare('INSERT OR IGNORE INTO reader_feeds (url, first_at, last_at) VALUES (?, ?, ?)').bind(url, now, now).run()
    count++
    fresh.push(url)
  }
  if (fresh.length && env.GITHUB_TOKEN) {
    await env.DB.batch(
      fresh.map((site) =>
        env.DB.prepare(
          'INSERT INTO refreshes (site, requested_at) VALUES (?, ?) ON CONFLICT (site) DO UPDATE SET requested_at = excluded.requested_at',
        ).bind(site, now),
      ),
    )
    ctx.waitUntil(dispatchRefreshes(env))
  }
  // Which blog each is: in the site's files (data/own/), not known here.
  return json({ blogs: {}, added: fresh.length })
}

/** A blog of the site (`example.com/blog`) or a reader's feed, to be read again. */
async function refresh(request, env, ctx) {
  if (!env.GITHUB_TOKEN) return error(503, '这里还不能请求刷新')
  const body = await request.json().catch(() => null)
  const site = typeof body?.site === 'string' ? body.site.trim().replace(/\/+$/, '') : ''
  await ready(env)
  let known = false
  if (/^https?:\/\//.test(site)) {
    known = !!(await env.DB.prepare('SELECT 1 AS y FROM reader_feeds WHERE url = ?').bind(site).first('y'))
  } else if (/^[a-z0-9.-]+(\/[^\s?#]*)?$/i.test(site)) {
    // A blog of the site has its page.
    const page = await env.ASSETS.fetch(new Request(new URL(`/blog/${site}/`, request.url)))
    known = page.status === 200
  }
  if (!known) return error(404, '找不到这个博客')
  const now = Math.floor(Date.now() / 1000)
  const row = await env.DB.prepare('SELECT requested_at, dispatched_at FROM refreshes WHERE site = ?').bind(site).first()
  if (row?.dispatched_at && now - row.dispatched_at < REFRESH_SECS) {
    return json({ status: 'later', error: '刚请求过', retry_at: row.dispatched_at + REFRESH_SECS }, 429)
  }
  await env.DB.prepare(
    'INSERT INTO refreshes (site, requested_at) VALUES (?, ?) ON CONFLICT (site) DO UPDATE SET requested_at = excluded.requested_at',
  )
    .bind(site, now)
    .run()
  const wait = Math.max(0, (await getState(env, 'refresh_dispatched')) + REFRESH_GAP - now)
  ctx.waitUntil(dispatchRefreshes(env))
  return json({ status: 'queued', eta_minutes: Math.ceil(wait / 60) + 5 })
}

/** One short crawl run for every blog asked for and not yet sent, when the limits allow. */
async function dispatchRefreshes(env) {
  const now = Math.floor(Date.now() / 1000)
  if (now - (await getState(env, 'refresh_dispatched')) < REFRESH_GAP) return
  const hour = Math.floor(now / 3600)
  const used = (await getState(env, 'refresh_hour')) === hour ? await getState(env, 'refresh_count') : 0
  if (used >= REFRESHES_PER_HOUR) return
  const { results } = await env.DB.prepare(
    'SELECT site FROM refreshes WHERE dispatched_at IS NULL OR dispatched_at < requested_at ORDER BY requested_at LIMIT ?',
  )
    .bind(SITES_PER_RUN)
    .all()
  if (!results.length) return
  const sites = results.map((r) => r.site)
  await env.DB.batch([
    ...sites.map((s) => env.DB.prepare('UPDATE refreshes SET dispatched_at = ? WHERE site = ?').bind(now, s)),
  ])
  await setState(env, 'refresh_dispatched', now)
  await setState(env, 'refresh_hour', hour)
  await setState(env, 'refresh_count', used + 1)
  await dispatch(env, { crawl: 'true', sites: sites.join(',') })
}

/** Starts the crawl workflow (.github/workflows/pages.yml) of the code repository. */
async function dispatch(env, inputs) {
  const res = await fetch(
    // GITHUB_API: only to try it out locally against a stand-in.
    `${env.GITHUB_API || 'https://api.github.com'}/repos/${env.GITHUB_REPO}/actions/workflows/pages.yml/dispatches`,
    {
      method: 'POST',
      headers: {
        authorization: `Bearer ${env.GITHUB_TOKEN}`,
        accept: 'application/vnd.github+json',
        'x-github-api-version': '2022-11-28',
        'user-agent': 'blogscroll-worker',
        'content-type': 'application/json',
      },
      body: JSON.stringify({ ref: env.GITHUB_REF || 'main', inputs }),
    },
  )
  if (!res.ok) console.error('dispatch failed', res.status, await res.text())
  return res.ok
}
