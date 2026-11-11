// Reading the feeds a reader adds for themselves (sources.ts), in the
// browser: RSS 2.0 and 1.0, Atom, JSON Feed, and OPML lists of feeds. Only
// what a card needs: title, link, date, a plain excerpt, a picture, tags.
// Pages parsed here (DOMParser) never run scripts or load anything.

export interface ParsedItem {
  /** The feed's own id for it (guid / id), else its link. */
  guid: string
  title: string
  url: string
  excerpt: string
  cover: string | null
  author: string | null
  tags: string[]
  /** Unix seconds; null when the feed gives no date. */
  published: number | null
  comments: number | null
  commentsUrl: string | null
}

export interface ParsedFeed {
  title: string
  homepage: string
  items: ParsedItem[]
}

export interface ListedFeed {
  url: string
  title: string
  homepage: string
}

export interface ParsedList {
  title: string
  feeds: ListedFeed[]
}

/** As long as the site's own excerpts (feed.rs EXCERPT_CHARS). */
const EXCERPT_CHARS = 240
const MAX_ITEMS = 30

const NS = {
  content: 'http://purl.org/rss/1.0/modules/content/',
  dc: 'http://purl.org/dc/elements/1.1/',
  slash: 'http://purl.org/rss/1.0/modules/slash/',
  media: 'http://search.yahoo.com/mrss/',
  itunes: 'http://www.itunes.com/dtds/podcast-1.0.dtd',
  atom: 'http://www.w3.org/2005/Atom',
}
/** Elements of RSS itself: no namespace (2.0) or RSS 1.0's. */
const RSS = [null, 'http://purl.org/rss/1.0/']

/** Bytes as the site's server passed them on → text, in the feed's own encoding (GBK too). */
export function decode(bytes: ArrayBuffer, contentType: string | null): string {
  let charset = /charset=["']?([\w.:-]+)/i.exec(contentType ?? '')?.[1]
  if (!charset) {
    const head = new TextDecoder('latin1').decode(bytes.slice(0, 1024))
    charset = /^\s*<\?xml[^>]*encoding=["']([\w.:-]+)["']/i.exec(head.replace(/^﻿/, ''))?.[1]
  }
  try {
    return new TextDecoder(charset || 'utf-8').decode(bytes)
  } catch {
    return new TextDecoder().decode(bytes)
  }
}

// Named entities of HTML that feeds use but XML does not know.
const ENTITIES: Record<string, number> = {
  nbsp: 160, copy: 169, reg: 174, trade: 8482, hellip: 8230, mdash: 8212, ndash: 8211, middot: 183,
  laquo: 171, raquo: 187, ldquo: 8220, rdquo: 8221, lsquo: 8216, rsquo: 8217, times: 215, deg: 176,
  bull: 8226, emsp: 8195, ensp: 8194, thinsp: 8201, yen: 165, euro: 8364, pound: 163, sect: 167,
}

function parseXml(text: string): Document {
  const parser = new DOMParser()
  const broken = (d: Document) => d.getElementsByTagName('parsererror').length > 0
  const doc = parser.parseFromString(text, 'application/xml')
  if (!broken(doc)) return doc
  // The usual slips: a bare &, HTML entities, control characters.
  const fixed = text
    .replace(/&(?!(?:#\d+|#x[\da-f]+|amp|lt|gt|quot|apos);)(\w+;)?/gi, (_, name?: string) => {
      const code = name && ENTITIES[name.slice(0, -1).toLowerCase()]
      return code ? `&#${code};` : `&amp;${name ?? ''}`
    })
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F]/g, '')
  const again = parser.parseFromString(fixed, 'application/xml')
  if (broken(again)) throw new Error('格式有误，读不出来')
  return again
}

const kids = (el: Element, name: string, ns: (string | null)[] = RSS) =>
  [...el.children].filter((c) => c.localName.toLowerCase() === name.toLowerCase() && ns.includes(c.namespaceURI))
const kid = (el: Element, name: string, ns?: (string | null)[]) => kids(el, name, ns)[0] as Element | undefined
const text = (el?: Element | null) => el?.textContent?.trim() ?? ''

function absolute(raw: string | null | undefined, base: string): string {
  if (!raw) return ''
  try {
    const u = new URL(raw.trim(), base)
    return u.protocol === 'http:' || u.protocol === 'https:' ? u.href : ''
  } catch {
    return ''
  }
}

function seconds(raw: string): number | null {
  if (!raw) return null
  const ms = Date.parse(raw.trim())
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null
}

const html = (s: string) => new DOMParser().parseFromString(s, 'text/html')

/** HTML → one line of text, cut to the excerpt's length. */
export function excerptOf(markup: string): string {
  const t = (html(markup).body.textContent ?? '').replace(/\s+/g, ' ').trim()
  return t.length > EXCERPT_CHARS ? `${t.slice(0, EXCERPT_CHARS).trimEnd()}…` : t
}

const NOT_PICTURES = /emoji|smil(ie|ey)|gravatar|feedburner|feedsportal|\/~r\/|pixel|1x1|spacer|badge|icon/i

/** The first picture in the HTML that is likely the post's, not an icon. */
function firstImage(markup: string, base: string): string | null {
  for (const img of html(markup).querySelectorAll('img')) {
    const src = absolute(img.getAttribute('data-src') || img.getAttribute('src'), base)
    const w = Number(img.getAttribute('width'))
    const h = Number(img.getAttribute('height'))
    if (!src || NOT_PICTURES.test(src) || (w && w < 80) || (h && h < 60)) continue
    return src
  }
  return null
}

function item(fields: Omit<ParsedItem, 'excerpt' | 'cover'> & { markup: string; picture: string | null }): ParsedItem {
  const { markup, picture, ...rest } = fields
  return {
    ...rest,
    title: excerptOf(rest.title).replace(/…$/, '') || '（无标题）',
    excerpt: excerptOf(markup),
    cover: picture ?? firstImage(markup, rest.url),
  }
}

function rssPicture(el: Element, base: string): string | null {
  const media = [...kids(el, 'thumbnail', [NS.media]), ...kids(el, 'content', [NS.media])].find(
    (m) => m.localName === 'thumbnail' || m.getAttribute('medium') === 'image' || m.getAttribute('type')?.startsWith('image/'),
  )
  const enclosure = kids(el, 'enclosure').find((e) => e.getAttribute('type')?.startsWith('image/'))
  const itunes = kid(el, 'image', [NS.itunes])
  return absolute(media?.getAttribute('url') ?? enclosure?.getAttribute('url') ?? itunes?.getAttribute('href'), base) || null
}

function parseRss(channel: Element, items: Element[], base: string): ParsedFeed {
  const atomLink = kids(channel, 'link', [NS.atom]).find((l) => (l.getAttribute('rel') ?? 'alternate') === 'alternate')
  const homepage = absolute(text(kid(channel, 'link')) || atomLink?.getAttribute('href'), base) || new URL(base).origin
  return {
    title: text(kid(channel, 'title')),
    homepage,
    items: items.slice(0, MAX_ITEMS).map((el) => {
      const guid = kid(el, 'guid')
      const permalink = guid?.getAttribute('isPermaLink') !== 'false' ? text(guid) : ''
      const url = absolute(text(kid(el, 'link')) || (/^https?:/.test(permalink) ? permalink : ''), homepage)
      const summary = text(kid(el, 'description'))
      const content = text(kid(el, 'encoded', [NS.content]))
      const comments = Number(text(kid(el, 'comments', [NS.slash])))
      return item({
        guid: text(guid) || url || text(kid(el, 'title')),
        title: text(kid(el, 'title')),
        url,
        markup: summary || content,
        picture: rssPicture(el, url || homepage) ?? (summary && content ? firstImage(content, url || homepage) : null),
        author: text(kid(el, 'creator', [NS.dc])) || text(kid(el, 'author')) || null,
        tags: kids(el, 'category').map(text).filter(Boolean).slice(0, 8),
        published: seconds(text(kid(el, 'pubDate'))) ?? seconds(text(kid(el, 'date', [NS.dc]))),
        comments: Number.isInteger(comments) && text(kid(el, 'comments', [NS.slash])) ? comments : null,
        commentsUrl: absolute(text(kid(el, 'comments', [null])), url || homepage) || null,
      })
    }),
  }
}

/** Atom text constructs: `html` is escaped markup, `xhtml` is markup itself. */
function atomText(el?: Element): string {
  if (!el) return ''
  return el.getAttribute('type') === 'xhtml' ? el.innerHTML : text(el)
}

function parseAtom(feed: Element, base: string): ParsedFeed {
  const A = [NS.atom, null]
  const alternate = (el: Element) =>
    kids(el, 'link', A).find((l) => (l.getAttribute('rel') ?? 'alternate') === 'alternate')?.getAttribute('href')
  const homepage = absolute(alternate(feed), base) || new URL(base).origin
  return {
    title: excerptOf(atomText(kid(feed, 'title', A))),
    homepage,
    items: kids(feed, 'entry', A)
      .slice(0, MAX_ITEMS)
      .map((el) => {
        const url = absolute(alternate(el), homepage)
        const summary = atomText(kid(el, 'summary', A))
        const content = atomText(kid(el, 'content', A))
        const enclosure = kids(el, 'link', A).find(
          (l) => l.getAttribute('rel') === 'enclosure' && l.getAttribute('type')?.startsWith('image/'),
        )
        return item({
          guid: text(kid(el, 'id', A)) || url,
          title: atomText(kid(el, 'title', A)),
          url,
          markup: summary || content,
          picture:
            rssPicture(el, url || homepage) ??
            (absolute(enclosure?.getAttribute('href'), url || homepage) || null) ??
            (summary && content ? firstImage(content, url || homepage) : null),
          author: text(kid(kid(el, 'author', A) ?? el, 'name', A)) || null,
          tags: kids(el, 'category', A)
            .map((c) => c.getAttribute('term') ?? '')
            .filter(Boolean)
            .slice(0, 8),
          published: seconds(text(kid(el, 'published', A))) ?? seconds(text(kid(el, 'updated', A))),
          comments: null,
          commentsUrl: null,
        })
      }),
  }
}

interface JsonFeed {
  title?: string
  home_page_url?: string
  items?: {
    id?: string | number
    url?: string
    title?: string
    content_html?: string
    content_text?: string
    summary?: string
    image?: string
    banner_image?: string
    date_published?: string
    date_modified?: string
    authors?: { name?: string }[]
    author?: { name?: string }
    tags?: string[]
  }[]
}

function parseJsonFeed(f: JsonFeed, base: string): ParsedFeed {
  const homepage = absolute(f.home_page_url, base) || new URL(base).origin
  return {
    title: f.title ?? '',
    homepage,
    items: (f.items ?? []).slice(0, MAX_ITEMS).map((it) => {
      const url = absolute(it.url, homepage)
      const escape = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;')
      return item({
        guid: String(it.id ?? url),
        title: escape(it.title ?? ''),
        url,
        markup: it.summary ? escape(it.summary) : (it.content_html ?? escape(it.content_text ?? '')),
        picture: absolute(it.image ?? it.banner_image, url || homepage) || null,
        author: it.authors?.[0]?.name ?? it.author?.name ?? null,
        tags: (it.tags ?? []).filter((t) => typeof t === 'string').slice(0, 8),
        published: seconds(it.date_published ?? '') ?? seconds(it.date_modified ?? ''),
        comments: null,
        commentsUrl: null,
      })
    }),
  }
}

/** A feed or a list of feeds (OPML), from its text; `base` is where it came from. */
export function parseSource(source: string, base: string): { feed: ParsedFeed } | { list: ParsedList } {
  if (source.trimStart().startsWith('{')) {
    let json: JsonFeed
    try {
      json = JSON.parse(source)
    } catch {
      throw new Error('格式有误，读不出来')
    }
    return { feed: parseJsonFeed(json, base) }
  }
  const doc = parseXml(source)
  const root = doc.documentElement
  switch (root.localName.toLowerCase()) {
    case 'rss': {
      const channel = kid(root, 'channel') ?? root
      return { feed: parseRss(channel, kids(channel, 'item'), base) }
    }
    // RSS 1.0: the items sit beside the channel.
    case 'rdf':
      return { feed: parseRss(kid(root, 'channel') ?? root, kids(root, 'item'), base) }
    case 'feed':
      return { feed: parseAtom(root, base) }
    case 'opml':
      return { list: parseOpml(doc, base) }
  }
  throw new Error('不是 RSS、Atom 或 OPML')
}

function parseOpml(doc: Document, base: string): ParsedList {
  const seen = new Set<string>()
  const feeds: ListedFeed[] = []
  for (const o of doc.getElementsByTagName('outline')) {
    const url = absolute(o.getAttribute('xmlUrl') ?? o.getAttribute('xmlurl'), base)
    // Feeds only an RSSHub server can read (rsshub://...) are left out.
    if (!url || seen.has(url)) continue
    seen.add(url)
    feeds.push({
      url,
      title: o.getAttribute('title') || o.getAttribute('text') || new URL(url).hostname,
      homepage: absolute(o.getAttribute('htmlUrl') ?? o.getAttribute('htmlurl'), url),
    })
  }
  return { title: text(doc.querySelector('head > title')), feeds }
}

/** An OPML file the reader picked. */
export function parseOpmlFile(source: string): ParsedList {
  const parsed = parseSource(source, location.href)
  if (!('list' in parsed)) throw new Error('这个文件不是 OPML 订阅列表')
  return parsed.list
}
