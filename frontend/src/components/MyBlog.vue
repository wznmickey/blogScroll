<script setup lang="ts">
// For bloggers: how crawling their blog goes, and where to ask for changes
// (an issue, or a pull request editing the site's config on GitHub).
import { computed, ref, watch } from 'vue'
import { api, meta } from '../api'
import { blogPath, fullDate, hostOf, relativeTime } from '../format'
import type { BlogDetail, BlogSummary } from '../types'
import BlogAvatar from './BlogAvatar.vue'
import SocialLinks from './SocialLinks.vue'

const props = defineProps<{ /** From a link such as /me?tab=blog&blog=4. */ initial?: number | null }>()

const KEY = 'my-blog'
function saved(): number | null {
  try {
    const id = Number(localStorage.getItem(KEY))
    return Number.isInteger(id) && id > 0 ? id : null
  } catch {
    return null
  }
}
const blogId = ref<number | null>(props.initial ?? saved())
function choose(id: number | null) {
  blogId.value = id
  try {
    if (id) localStorage.setItem(KEY, String(id))
    else localStorage.removeItem(KEY)
  } catch {
    // Remembered for this visit only.
  }
}

// Finding one's blog.
const q = ref('')
const results = ref<BlogSummary[] | null>(null)
const searching = ref(false)
async function search() {
  const text = q.value.trim()
  if (!text) return
  searching.value = true
  try {
    // An address is searched by its host: "https://www.a.com/blog/" finds a.com.
    results.value = (await api.blogs({ q: /^https?:\/\//.test(text) ? hostOf(text) : text, limit: 8 })).items
  } catch {
    results.value = []
  } finally {
    searching.value = false
  }
}

const blog = ref<BlogDetail | null>(null)
const missing = ref(false)
watch(
  blogId,
  async (id) => {
    blog.value = null
    missing.value = false
    if (!id) return
    try {
      blog.value = await api.blog(id)
    } catch {
      missing.value = true
    }
  },
  { immediate: true },
)

const crawl = computed(() => blog.value?.crawl ?? null)
const configured = (key: string) => crawl.value?.configured.includes(key) ?? false

function later(t: number | null | undefined): string {
  if (!t) return ''
  const diff = t - Date.now() / 1000
  if (diff <= 60) return '马上'
  if (diff < 3600) return `${Math.ceil(diff / 60)} 分钟后`
  if (diff < 86400) return `${Math.round(diff / 3600)} 小时后`
  return `${Math.round(diff / 86400)} 天后`
}

const FEED_ORIGIN: Record<string, string> = {
  directory: '目录里登记的',
  discovered: '从你的网站上找到的',
  manual: '站长按你的要求设置的',
}

const aboutUse = computed(() => {
  const b = blog.value
  if (!b) return ''
  if (b.about_html) return '整页显示'
  if (b.about_text) return `显示一段介绍：「${[...b.about_text].slice(0, 30).join('')}…」`
  return '没有读到介绍'
})

const avatarSource = computed(() => {
  const c = crawl.value
  if (!c || !blog.value) return ''
  if (configured('avatar')) return '你在配置里指定的'
  if (c.found_avatar) return '从你的网站上找到的'
  if (c.feed_image) return 'RSS 里的图片'
  return '没找到，显示名字的首字'
})

// Asking for changes.
const repo = computed(() => meta.value?.repo ?? null)
const editUrl = computed(() => (repo.value ? `${repo.value.url}/edit/main/${repo.value.config_path}` : ''))
const issueUrl = computed(() => {
  const b = blog.value
  if (!repo.value || !b) return ''
  const c = b.crawl
  const lines = [
    `博客：${b.name}`,
    `地址：${b.homepage}`,
    `本站页面：${location.origin}${import.meta.env.BASE_URL}blog/${b.id}`,
    '',
    '想修改的内容（头像 / 简介 / 社交链接 / 标签 / 关于页 / RSS / 授权全文 / 嵌入评论 / 不再收录）：',
    '',
    '',
    '目前抓取到的：',
    `- RSS：${c?.feed_url ?? '没有找到'}`,
    `- 头像：${b.icon ?? '没有'}`,
    `- 简介：${b.description ?? '没有'}`,
    `- 社交链接：${b.social.map((s) => s.url).join(' ') || '没有'}`,
    `- 标签：${b.top_tags.join('、') || '没有'}`,
  ]
  return `${repo.value.url}/issues/new?title=${encodeURIComponent(`博客资料：${b.name}`)}&body=${encodeURIComponent(lines.join('\n'))}`
})

const q2 = (s: string) => JSON.stringify(s)
/** A config section to paste, filled with what is shown now. */
const snippet = computed(() => {
  const b = blog.value
  if (!b) return ''
  return [
    '[[policy.profile]]',
    `site = ${q2(b.homepage)}`,
    `${configured('avatar') ? '' : '# '}avatar = ${q2(b.icon ?? 'https://…/avatar.png')}`,
    `${configured('description') ? '' : '# '}description = ${q2(b.description ?? '一句话介绍')}`,
    `${configured('social') ? '' : '# '}social = [${b.social.map((s) => q2(s.url)).join(', ')}]`,
    `${configured('tags') ? '' : '# '}tags = [${b.top_tags.map(q2).join(', ')}]`,
    `${configured('about') ? '' : '# '}about = ${q2(b.crawl?.about_page ?? `${b.homepage.replace(/\/$/, '')}/about/`)}   # 关于页地址`,
    `${b.about_html ? '' : '# '}about_full = true   # 关于页整页显示（保留排版），而不是约 300 字摘要`,
    `# about_text = "自己写的介绍，可以多行。"   # 或者直接用这段介绍`,
    `note = "博主本人申请"`,
  ].join('\n')
})
const copied = ref(false)
async function copy() {
  try {
    await navigator.clipboard.writeText(snippet.value)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    copied.value = false
  }
}
</script>

<template>
  <section class="my-blog">
    <template v-if="!blogId">
      <div class="card">
        <h2>你是博主吗？</h2>
        <p>找到你的博客，就能看到本站抓取它的情况：RSS、上次抓取、头像、社交链接、标签……哪里不对，可以直接告诉站长。</p>
        <form class="find" @submit.prevent="search">
          <input v-model="q" placeholder="博客名或网址" aria-label="博客名或网址" />
          <button class="btn btn-primary" type="submit" :disabled="searching">查找</button>
        </form>
        <ul v-if="results?.length" class="hits">
          <li v-for="r in results" :key="r.id">
            <BlogAvatar :name="r.name" :icon="r.icon" :size="36" />
            <span class="hit-text">
              <b>{{ r.name }}</b>
              <span class="sub">{{ hostOf(r.homepage) }}</span>
            </span>
            <button class="btn btn-ghost" @click="choose(r.id)">这是我的</button>
          </li>
        </ul>
        <p v-else-if="results" class="note">
          没有找到。本站收录<a href="https://www.travellings.cn" target="_blank" rel="noopener">开往</a>的成员，以及站长另外订阅的博客<template
            v-if="repo"
            >；想被收录，可以<a :href="`${repo.url}/issues/new?title=${encodeURIComponent('申请收录')}`" target="_blank" rel="noopener"
              >在 GitHub 提 issue</a
            ></template
          >。
        </p>
      </div>
    </template>

    <template v-else>
      <p v-if="missing" class="card note">
        没有读到这个博客：可能已经不再收录，或者暂时没有内容。<button class="link" @click="choose(null)">换一个</button>
      </p>
      <p v-else-if="!blog" class="empty"><span class="spinner" /></p>
      <template v-else>
        <div class="card who">
          <BlogAvatar :name="blog.name" :icon="blog.icon" :size="48" />
          <span class="hit-text">
            <RouterLink :to="blogPath(blog)"><b>{{ blog.name }}</b></RouterLink>
            <a class="sub" :href="blog.homepage" target="_blank" rel="noopener">{{ hostOf(blog.homepage) }} ↗</a>
          </span>
          <button class="link" @click="choose(null)">不是我的</button>
        </div>

        <div class="card">
          <h2>抓取</h2>
          <p v-if="!crawl" class="note">这个站点的数据还不含抓取状态。</p>
          <dl v-else>
            <dt>收录于</dt>
            <dd>{{ blog.directories.map((d) => d.name).join('、') || '站长另外订阅' }}</dd>
            <dt>RSS</dt>
            <dd v-if="crawl.feed_url">
              <a :href="crawl.feed_url" target="_blank" rel="noopener" class="mono">{{ crawl.feed_url }}</a>
              <span class="sub">{{ FEED_ORIGIN[crawl.feed_origin ?? ''] ?? '' }}</span>
            </dd>
            <dd v-else>
              还没找到。<span v-if="crawl.discovery_at">上次在 {{ relativeTime(crawl.discovery_at) }} 找过，</span>{{ later(crawl.next_check_at) }}再找。
              在首页的 <code>&lt;head&gt;</code> 里加
              <code>&lt;link rel="alternate" type="application/rss+xml" href="…"&gt;</code> 就能被找到。
            </dd>
            <dt>上次检查</dt>
            <dd>
              {{ crawl.last_checked_at ? relativeTime(crawl.last_checked_at) : '还没有' }}
              <span v-if="crawl.last_success_at" class="sub">（上次成功 {{ relativeTime(crawl.last_success_at) }}）</span>
            </dd>
            <dt>下次检查</dt>
            <dd>{{ later(crawl.next_check_at) }}</dd>
            <template v-if="crawl.failures">
              <dt>出错</dt>
              <dd class="bad">连续 {{ crawl.failures }} 次：{{ crawl.last_error }}</dd>
            </template>
            <dt>文章</dt>
            <dd>
              {{ blog.post_count }} 篇<span v-if="blog.last_post_at">，最近一篇 {{ relativeTime(blog.last_post_at) }}</span>
            </dd>
            <dt>内容</dt>
            <dd>{{ blog.full_text ? '你已授权，保存全文' : '只保存 RSS 里的摘要，读全文会去你的网站' }}</dd>
            <dt>评论</dt>
            <dd>
              {{ blog.comment_system ?? '没认出评论系统' }}<span v-if="blog.comments_embedded">，你已授权在本站显示</span>
            </dd>
          </dl>
        </div>

        <div v-if="crawl" class="card">
          <h2>资料</h2>
          <p class="note">每月从你的首页和关于页读一次{{ crawl.profile_at ? `，上次是 ${fullDate(crawl.profile_at)}` : '' }}。</p>
          <dl>
            <dt>头像<span v-if="configured('avatar')" class="tag">配置</span></dt>
            <dd class="row">
              <BlogAvatar :name="blog.name" :icon="blog.icon" :size="40" />
              <span class="sub">{{ avatarSource }}</span>
            </dd>
            <dt>简介<span v-if="configured('description')" class="tag">配置</span></dt>
            <dd>{{ blog.description || '没有（读的是首页的 meta description）' }}</dd>
            <dt>社交链接<span v-if="configured('social')" class="tag">配置</span></dt>
            <dd>
              <SocialLinks v-if="blog.social.length" :links="blog.social" />
              <span v-else>没找到</span>
            </dd>
            <dt>关于页<span v-if="configured('about')" class="tag">配置</span></dt>
            <dd>
              <a v-if="crawl.about_page" :href="crawl.about_page" target="_blank" rel="noopener" class="mono">{{
                crawl.about_page
              }}</a>
              <span v-else>没找到</span>
              <span class="sub">{{ aboutUse }}</span>
            </dd>
            <dt>标签<span v-if="configured('tags')" class="tag">配置</span></dt>
            <dd>{{ blog.top_tags.map((t) => `#${t}`).join(' ') || '还没有' }}</dd>
          </dl>
        </div>

        <div class="card">
          <h2>需要修改？</h2>
          <p>
            头像、简介、社交链接、标签、关于页认错了，想授权全文或在本站显示评论，或者不想被收录，都可以告诉站长。
          </p>
          <template v-if="repo">
            <p class="actions">
              <a class="btn btn-primary" :href="issueUrl" target="_blank" rel="noopener">在 GitHub 提 issue</a>
              <a class="btn btn-ghost" :href="editUrl" target="_blank" rel="noopener">直接改配置提 PR</a>
            </p>
            <p class="note">
              提 PR：在 <code>{{ repo.config_path }}</code> 末尾加上下面这段（去掉要改的行前面的 <code>#</code>，写上你想要的），
              GitHub 会帮你建好分支和 PR。授权全文、显示评论、不再收录也在这个文件里（<code>policy.full_text</code>、<code
                >policy.embed_comments</code
              >、<code>policy.exclude</code>）。
            </p>
            <div class="snippet">
              <pre>{{ snippet }}</pre>
              <button class="btn btn-ghost copy" @click="copy">{{ copied ? '已复制' : '复制' }}</button>
            </div>
          </template>
          <p v-else class="note">请通过<RouterLink to="/about">关于</RouterLink>页上的方式联系站长。</p>
        </div>
      </template>
    </template>
  </section>
</template>

<style scoped>
.my-blog {
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 760px;
}
.card {
  padding: 16px 18px;
  border-radius: 14px;
  background: var(--surface);
  min-width: 0;
}
.card h2 {
  margin: 0 0 8px;
  font-size: 16px;
}
.card p {
  margin: 0 0 10px;
  line-height: 1.6;
}
.note,
.sub {
  color: var(--text-2);
  font-size: 13px;
}
.find {
  display: flex;
  gap: 8px;
}
.find input {
  flex: 1;
  min-width: 0;
  height: 38px;
  padding: 0 14px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--bg);
  color: var(--text);
  font: inherit;
}
.hits {
  margin: 12px 0 0;
  padding: 0;
  list-style: none;
}
.hits li,
.who {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 0;
}
.card.who {
  padding: 12px 18px;
}
.hit-text {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}
.hit-text > * {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
dl {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 8px 16px;
  margin: 0;
  font-size: 14px;
}
dt {
  color: var(--text-2);
}
dd {
  margin: 0;
  min-width: 0;
  overflow-wrap: anywhere;
}
dd.row {
  display: flex;
  align-items: center;
  gap: 10px;
}
dd :deep(.social) {
  margin: 0;
}
.mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
}
code {
  padding: 0 4px;
  border-radius: 4px;
  background: var(--bg);
  font-size: 12px;
  overflow-wrap: anywhere;
}
.bad {
  color: var(--accent);
}
.tag {
  margin-left: 6px;
  padding: 0 6px;
  border-radius: 999px;
  background: var(--accent-soft);
  color: var(--accent);
  font-size: 11px;
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.snippet {
  position: relative;
}
.snippet pre {
  margin: 0;
  padding: 12px 14px;
  border-radius: 10px;
  background: var(--bg);
  font-size: 12px;
  line-height: 1.6;
  overflow-x: auto;
}
.copy {
  position: absolute;
  top: 6px;
  right: 6px;
}
a:not(.btn) {
  color: var(--link);
}
dd .sub {
  margin-left: 8px;
}
@media (max-width: 639px) {
  dl {
    grid-template-columns: minmax(0, 1fr);
    gap: 2px;
  }
  dd {
    margin-bottom: 8px;
  }
}
</style>
