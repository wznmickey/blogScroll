const SOCIAL: Record<string, string> = {
  github: 'GitHub',
  gitee: 'Gitee',
  bilibili: 'B站',
  weibo: '微博',
  zhihu: '知乎',
  x: 'X',
  telegram: 'Telegram',
  instagram: 'Instagram',
  youtube: 'YouTube',
  douban: '豆瓣',
  xiaohongshu: '小红书',
  link: '个人主页',
}

export const socialLabel = (kind: string) => SOCIAL[kind] ?? kind
