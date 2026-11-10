const SOCIAL: Record<string, string> = {
  github: 'GitHub',
  gitee: 'Gitee',
  gitlab: 'GitLab',
  bilibili: 'B站',
  weibo: '微博',
  zhihu: '知乎',
  x: 'X',
  telegram: 'Telegram',
  instagram: 'Instagram',
  youtube: 'YouTube',
  douban: '豆瓣',
  xiaohongshu: '小红书',
  douyin: '抖音',
  steam: 'Steam',
  pixiv: 'pixiv',
  bangumi: 'Bangumi',
  juejin: '掘金',
  csdn: 'CSDN',
  v2ex: 'V2EX',
  coolapk: '酷安',
  bluesky: 'Bluesky',
  linkedin: '领英',
  facebook: 'Facebook',
  threads: 'Threads',
  twitch: 'Twitch',
  netease_music: '网易云音乐',
  fediverse: '联邦宇宙',
  link: '个人主页',
}

export const socialLabel = (kind: string) => SOCIAL[kind] ?? kind

// Path segments before the account's name: zhihu.com/people/<name>, ...
const PREFIXES = new Set(['people', 'user', 'users', 'u', 'profile', 'profiles', 'in', 'id', 'member', 'space', 'channel', 'c'])

/** The account's name in its URL: "wznmickeyEn" in https://x.com/wznmickeyEn
 *  (a personal site: its host). */
function handleOf(kind: string, url: string): string {
  try {
    const u = new URL(url)
    const host = u.hostname.replace(/^www\./, '')
    if (kind === 'link') return host
    // NetEase Music: #/user/home?id=<id>
    if (u.hostname === 'music.163.com') return new URLSearchParams(u.hash.split('?')[1] ?? '').get('id') ?? ''
    const seg = u.pathname
      .split('/')
      .filter(Boolean)
      .find((s) => !PREFIXES.has(s))
    return seg ? decodeURIComponent(seg) : host
  } catch {
    return ''
  }
}

/** Names for a list of links: "X", and for more accounts on the same
 *  platform "X (wznmickeyEn)". */
export function socialLabels(links: { kind: string; url: string }[]): string[] {
  const seen = new Set<string>()
  return links.map(({ kind, url }) => {
    const label = socialLabel(kind)
    if (!seen.has(kind)) {
      seen.add(kind)
      return label
    }
    const handle = handleOf(kind, url)
    return handle ? `${label} (${handle})` : label
  })
}
