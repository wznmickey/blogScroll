// Icons for social links. Brand marks from Simple Icons (CC0); platforms it
// has none for get a lettered badge in their colour.

import {
  siBilibili,
  siBluesky,
  siCodeberg,
  siCsdn,
  siDouban,
  siFacebook,
  siGitee,
  siGithub,
  siGitlab,
  siInstagram,
  siJuejin,
  siMastodon,
  siMisskey,
  siNeteasecloudmusic,
  siPixiv,
  siSinaweibo,
  siSteam,
  siTelegram,
  siThreads,
  siTiktok,
  siTwitch,
  siV2ex,
  siX,
  siXiaohongshu,
  siYoutube,
  siZhihu,
  type SimpleIcon,
} from 'simple-icons'

export interface SocialIcon {
  /** A 24×24 SVG path, or a short text for platforms without a mark. */
  path?: string
  text?: string
  /** Brand colour, as #rrggbb. */
  color: string
}

const MARKS: Record<string, SimpleIcon> = {
  github: siGithub,
  gitee: siGitee,
  bilibili: siBilibili,
  weibo: siSinaweibo,
  zhihu: siZhihu,
  x: siX,
  telegram: siTelegram,
  instagram: siInstagram,
  youtube: siYoutube,
  douban: siDouban,
  xiaohongshu: siXiaohongshu,
  // Douyin shares TikTok's note.
  douyin: siTiktok,
  steam: siSteam,
  pixiv: siPixiv,
  juejin: siJuejin,
  csdn: siCsdn,
  v2ex: siV2ex,
  bluesky: siBluesky,
  facebook: siFacebook,
  threads: siThreads,
  twitch: siTwitch,
  netease_music: siNeteasecloudmusic,
}

const BADGES: Record<string, SocialIcon> = {
  linkedin: { text: 'in', color: '#0a66c2' },
  bangumi: { text: '番', color: '#f09199' },
  coolapk: { text: '酷', color: '#11a84e' },
}

// A personal site: a chain link (drawn for this site).
const LINK =
  'M10.6 13.4a1 1 0 0 1 0-1.4l3.6-3.6a3 3 0 1 1 4.2 4.2l-2 2a1 1 0 1 1-1.4-1.4l2-2a1 1 0 1 0-1.4-1.4l-3.6 3.6a1 1 0 0 1-1.4 0zm2.8-2.8a1 1 0 0 1 0 1.4l-3.6 3.6a3 3 0 1 1-4.2-4.2l2-2a1 1 0 0 1 1.4 1.4l-2 2a1 1 0 1 0 1.4 1.4l3.6-3.6a1 1 0 0 1 1.4 0z'

export function socialIcon(kind: string, url: string): SocialIcon {
  // GitLab and Codeberg share a kind.
  if (kind === 'gitlab') {
    const mark = url.includes('codeberg.org') ? siCodeberg : siGitlab
    return { path: mark.path, color: `#${mark.hex}` }
  }
  if (kind === 'fediverse') {
    const mark = /misskey|sharkey|firefish/i.test(url) ? siMisskey : siMastodon
    return { path: mark.path, color: `#${mark.hex}` }
  }
  const mark = MARKS[kind]
  if (mark) return { path: mark.path, color: `#${mark.hex}` }
  return BADGES[kind] ?? { path: LINK, color: '#5c5c66' }
}
