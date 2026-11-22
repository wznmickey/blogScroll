// RSSHub feeds are read from the site's own RSSHub instance: the public one
// (rsshub.app) turns feed readers away (HTTP 403, "intended for testing
// purposes only"), and rsshub:// addresses (Folo, RSSHub Radar) name a
// route, not a server. Rewritten wherever an address comes in: typed,
// from an OPML list, from storage, a backup or another device (sources.ts).

/** The site's RSSHub instance, on Vercel. */
export const RSSHUB = 'https://rss-hub-flax-iota.vercel.app'

/** `https://rsshub.app/zhihu/daily` or `rsshub://zhihu/daily` → the same route on RSSHUB; any other address as it is. */
export function viaOurRsshub(url: string): string {
  const m = /^(?:rsshub:\/\/|https?:\/\/(?:www\.)?rsshub\.app(?=[/?#]|$))\/?(.*)$/i.exec(url.trim())
  return m ? `${RSSHUB}/${m[1]}` : url
}
