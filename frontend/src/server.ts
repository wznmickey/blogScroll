// What the site's server offers besides the site itself (backend/src/readers.rs,
// or for the static site the Cloudflare Worker, worker/src/index.js): sync
// between a reader's devices, fetching the feeds they add themselves (and
// crawling them like the site's own), and reading a blog again on request.
// Plain static hosting (GitHub Pages) has none of it.

import { ref } from 'vue'
import { decode } from './feedParse'

export interface Capabilities {
  sync: boolean
  fetch: boolean
  /** Readers' own feeds are crawled too (POST /api/readers/feeds). */
  readers: boolean
  /** A blog can be read again on request (POST /api/refresh). */
  refresh: boolean
}

/** null until known. */
export const capabilities = ref<Capabilities | null>(null)

let asking: Promise<Capabilities> | null = null
export function loadCapabilities(): Promise<Capabilities> {
  asking ??= fetch('/api/capabilities', { cache: 'no-store' })
    .then(async (r) => {
      const c = r.ok && r.headers.get('content-type')?.includes('json') ? await r.json() : {}
      return (capabilities.value = {
        sync: c?.sync === true,
        fetch: c?.fetch === true,
        readers: c?.readers === true,
        refresh: c?.refresh === true,
      })
    })
    .catch(() => {
      // Offline: ask again next time.
      asking = null
      return { sync: false, fetch: false, readers: false, refresh: false }
    })
  return asking
}

export class FetchFailed extends Error {
  /** Seconds the site asked to wait (429 / 503). */
  retryAfter?: number
}

/** What /api/fetch found at an address: a feed or a list (as text), or the feeds a page links to. */
export type Fetched = { kind: 'feed'; text: string; url: string } | { kind: 'page'; feeds: string[]; url: string }

export async function fetchRemote(url: string): Promise<Fetched> {
  let res: Response
  try {
    res = await fetch(`/api/fetch?url=${encodeURIComponent(url)}`)
  } catch {
    throw new FetchFailed('连不上本站的服务器')
  }
  if (!res.ok) {
    let message = `HTTP ${res.status}`
    try {
      message = (await res.json()).error ?? message
    } catch {
      if (res.status === 404) message = '这里的服务器不能代为读取 RSS'
    }
    const e = new FetchFailed(message)
    const wait = Number(res.headers.get('retry-after'))
    if (wait > 0) e.retryAfter = wait
    throw e
  }
  if (res.headers.get('x-kind') === 'page') {
    const j = await res.json()
    return { kind: 'page', feeds: Array.isArray(j.feeds) ? j.feeds : [], url: j.final_url || url }
  }
  return {
    kind: 'feed',
    text: decode(await res.arrayBuffer(), res.headers.get('x-feed-type')),
    url: res.headers.get('x-final-url') || url,
  }
}
