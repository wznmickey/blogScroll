<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api'
import BlogAvatar from '../components/BlogAvatar.vue'
import FollowButton from '../components/FollowButton.vue'
import Waterfall from '../components/Waterfall.vue'
import SocialLinks from '../components/SocialLinks.vue'
import { blogPath, hostOf, relativeTime } from '../format'
import { recordShown } from '../library'
import { isBlocked, options, toggleBlock } from '../options'
import { openInReader, readerIntent, readerIntentEnd } from '../reader'
import { capabilities, loadCapabilities } from '../server'
import { crawledFeed, readNow, refreshOwn, removeOwn } from '../sources'
import type { PostCard } from '../types'
import type { BlogDetail } from '../types'

const route = useRoute()
const router = useRouter()
/** The blog's address (example.com/blog), or its number in links from before. */
const key = computed(() => String(route.params.key ?? '').replace(/\/+$/, ''))
const blog = ref<BlogDetail | null>(null)
const missing = ref(false)
const id = computed(() => blog.value?.id ?? 0)

watch(
  key,
  async (k) => {
    if (blog.value && (blog.value.key === k || String(blog.value.id) === k)) return
    blog.value = null
    missing.value = false
    try {
      const found = /^\d+$/.test(k) ? await api.blog(Number(k)) : await api.blogByKey(k)
      if (key.value !== k) return
      blog.value = found
      document.title = found.name
      // An old link by number: show the address instead.
      if (found.key && found.key !== k) router.replace({ path: blogPath(found), query: route.query, hash: route.hash })
    } catch {
      if (key.value === k) missing.value = true
    }
  },
  { immediate: true },
)

// The blog's own page shows its posts even if the reader blocked it (they came here).
const load = async (cursor: string | null) => {
  const page = await api.posts({ mode: 'latest', blog: id.value, cursor, nsfw: options.value.showNsfw || undefined })
  if (cursor !== null || !justRead.value.length) return page
  // Read just now, ahead of the site: on top, marked new.
  const have = new Set(page.items.map((p) => p.url))
  return { ...page, items: [...justRead.value.filter((p) => !have.has(p.url)), ...page.items] }
}

// 刷新: a visitor asks for the blog to be read again now. A server that
// crawls does it at once; the static site's Worker has the crawler run for it
// (a few minutes) and meanwhile the feed is read here, its new posts shown.
void loadCapabilities()
const refreshing = ref(false)
const refreshNote = ref<string | null>(null)
const justRead = ref<PostCard[]>([])
const pageKey = ref(0)
watch(id, () => {
  refreshNote.value = null
  justRead.value = []
})
const minutes = (at: number) => Math.max(1, Math.ceil((at - Date.now() / 1000) / 60))
async function refresh() {
  const b = blog.value
  if (!b || refreshing.value) return
  refreshing.value = true
  refreshNote.value = null
  try {
    // A feed of the reader's own that the site does not crawl: read here.
    if (b.id < 0) {
      await refreshOwn(true)
      refreshNote.value = '已重新读取'
      pageKey.value++
      return
    }
    const res = await fetch('/api/refresh', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ site: crawledFeed(b.id) ?? b.key ?? String(b.id) }),
    })
    const r = await res.json().catch(() => ({}))
    if (r.status === 'done') {
      refreshNote.value = r.new_posts ? `读到 ${r.new_posts} 篇新文章` : '读过了，没有新文章'
      pageKey.value++
    } else if (r.status === 'queued') {
      refreshNote.value = `已请求，约 ${r.eta_minutes ?? 5} 分钟后网站更新`
      const feed = b.crawl?.feed_url ?? crawledFeed(b.id)
      if (feed) {
        const newest = (await api.posts({ mode: 'latest', blog: b.id, limit: 1 })).items[0]?.sort_at ?? 0
        const ref = { id: b.id, key: b.key, name: b.name, homepage: b.homepage, icon: b.icon, kind: b.kind.id, framable: b.framable }
        justRead.value = (await readNow(feed, ref).catch(() => [])).filter((p) => p.sort_at > newest)
        if (justRead.value.length) {
          refreshNote.value += `；先显示刚读到的 ${justRead.value.length} 篇`
          pageKey.value++
        }
      }
    } else if (r.status === 'later' && r.retry_at) {
      refreshNote.value = `${r.error}，${minutes(r.retry_at)} 分钟后可以再刷新`
    } else {
      refreshNote.value = r.error ?? `没能刷新（HTTP ${res.status}）`
    }
  } catch {
    refreshNote.value = '没能刷新，稍后再试'
  } finally {
    refreshing.value = false
  }
}
const isFresh = (p: PostCard) => justRead.value.some((x) => x.id === p.id)

const aboutOpen = ref(false)
watch(id, () => (aboutOpen.value = false))

function openBeside(e: MouseEvent, url: string, title: string) {
  if (blog.value && openInReader({ url, title, blog: blog.value }, e)) e.preventDefault()
}

/** Resting on a link to the blog's site loads it in the (hidden) pane. */
function ahead(url: string, title: string, delay?: number) {
  if (blog.value) readerIntent({ url, title, blog: blog.value }, delay)
}

/** A feed of the reader's own, added on its own (not from a list): it can go. */
function unsubscribe() {
  if (!blog.value?.own) return
  removeOwn(blog.value.own.feed)
  router.push('/settings#sources')
}

/** "Hexo 7.3.0" → "Hexo" */
const engine = computed(() => blog.value?.generator?.split(/\s+/)[0] ?? null)
</script>

<template>
  <div class="container">
    <section v-if="blog" class="profile">
      <BlogAvatar :name="blog.name" :icon="blog.icon" :size="88" />
      <div class="info">
        <div class="title-row">
          <h1>{{ blog.name }}</h1>
          <FollowButton :blog-id="blog.id" />
          <button v-if="blog.own && !blog.own.list" type="button" class="block" @click="unsubscribe">移除订阅</button>
          <button
            v-else-if="!blog.own"
            type="button"
            class="block"
            :class="{ on: isBlocked(blog.id) }"
            :title="isBlocked(blog.id) ? '推荐、最新和搜索里又会出现这个博主' : '推荐、最新和搜索里不再出现这个博主（可在「设置」里撤销）'"
            @click="toggleBlock({ id: blog.id, name: blog.name })"
          >
            {{ isBlocked(blog.id) ? '已屏蔽' : '屏蔽' }}
          </button>
        </div>
        <a
          :href="blog.homepage"
          target="_blank"
          rel="noopener"
          class="home"
          @pointerenter="ahead(blog.homepage, blog.name)"
          @pointerleave="readerIntentEnd"
          @focus="ahead(blog.homepage, blog.name)"
          @pointerdown="ahead(blog.homepage, blog.name, 0)"
          @click="openBeside($event, blog.homepage, blog.name)"
          >{{ hostOf(blog.homepage) }} ↗</a
        >
        <button
          v-if="capabilities?.refresh || blog.id < 0"
          type="button"
          class="refresh"
          :disabled="refreshing"
          title="现在就去读一次这个博客的 RSS"
          @click="refresh"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" :class="{ spin: refreshing }">
            <path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7" />
          </svg>
          {{ refreshing ? '读取中…' : '刷新' }}
        </button>
        <span v-if="refreshNote" class="refresh-note">{{ refreshNote }}</span>
        <p v-if="blog.description" class="desc">{{ blog.description }}</p>
        <p class="chips">
          <span v-if="blog.kind && blog.kind.id !== 'blog'" class="chip source">{{ blog.kind.label }}</span>
          <span v-for="d in blog.directories" :key="d.directory" class="chip source">收录于{{ d.name }}</span>
          <span v-if="blog.own" class="chip source" :title="blog.own.feed">
            你添加的订阅{{ blog.own.list ? `，来自列表「${blog.own.list}」` : '' }}
          </span>
          <RouterLink
            v-for="c in blog.categories ?? []"
            :key="c.id"
            :to="{ path: '/', query: { c: `cat:${c.id}` } }"
            class="chip link topic"
            :title="c.share ? `约 ${Math.round(c.share * 100)}% 的文章` : undefined"
            >{{ c.label }}</RouterLink
          >
          <RouterLink
            v-for="t in blog.tags"
            :key="t.directory + t.tag"
            :to="{ path: '/', query: { dirtag: t.tag } }"
            class="chip link"
            >{{ t.label }}</RouterLink
          >
          <span v-if="blog.full_text" class="chip">作者已授权全文</span>
          <span v-if="engine" class="chip">{{ engine }}</span>
          <span v-if="blog.comment_system" class="chip">评论 · {{ blog.comment_system }}</span>
        </p>
        <p v-if="blog.top_tags?.length" class="top-tags">
          <RouterLink v-for="t in blog.top_tags" :key="t" :to="{ path: '/', query: { c: `tag:${t}` } }">#{{ t }}</RouterLink>
        </p>
        <SocialLinks v-if="blog?.social.length" :links="blog.social" />
        <p class="stats">
          <span><b>{{ blog.post_count }}</b> 篇文章</span>
          <span v-if="blog.last_post_at">最近更新 {{ relativeTime(blog.last_post_at) }}</span>
          <span v-if="!blog.has_feed">暂未找到 RSS</span>
          <span v-if="blog.own?.error" class="own-error">上次读取失败：{{ blog.own.error }}</span>
          <RouterLink v-if="!blog.own" :to="{ path: '/me', query: { tab: 'blog', blog: blog.id } }" class="mine">这是你的博客？查看抓取情况</RouterLink>
        </p>
      </div>
    </section>
    <section v-if="blog?.about_html || blog?.about_text" class="about">
      <h2>关于</h2>
      <!-- The whole page, for authors who asked for it (sanitised on the server). -->
      <template v-if="blog.about_html">
        <div class="about-html" :class="{ clamped: !aboutOpen }" v-html="blog.about_html" />
        <button type="button" class="more" @click="aboutOpen = !aboutOpen">{{ aboutOpen ? '收起' : '展开全部' }}</button>
      </template>
      <p v-else>{{ blog.about_text }}</p>
      <a
        v-if="blog.about_url"
        :href="blog.about_url"
        target="_blank"
        rel="noopener"
        @pointerenter="ahead(blog.about_url, `关于 · ${blog.name}`)"
        @pointerleave="readerIntentEnd"
        @focus="ahead(blog.about_url, `关于 · ${blog.name}`)"
        @pointerdown="ahead(blog.about_url, `关于 · ${blog.name}`, 0)"
        @click="openBeside($event, blog.about_url, `关于 · ${blog.name}`)"
        >阅读完整介绍 ↗</a
      >
    </section>
    <p v-else-if="missing" class="missing">找不到这个博客</p>

    <Waterfall
      v-if="blog"
      :load="load"
      :reset-key="`${id}|${pageKey}`"
      :is-fresh="isFresh"
      @impression="(p) => recordShown(p, blog?.name ?? '博客页')"
    >
      <template #empty>
        <p class="missing">{{ blog.own ? '还没有读到这个订阅的文章' : '暂时还没有抓取到这个博客的文章' }}</p>
      </template>
    </Waterfall>
  </div>
</template>

<style scoped>
.own-error {
  color: var(--accent);
}
.profile {
  display: flex;
  align-items: center;
  gap: 28px;
  max-width: 880px;
  margin: 24px auto 32px;
  padding: 0 8px;
}
.info {
  min-width: 0;
}
.title-row {
  display: flex;
  align-items: center;
  gap: 14px;
}
.block {
  padding: 4px 10px;
  border-radius: 999px;
  color: var(--text-3);
  font-size: 13px;
}
.block:hover {
  background: var(--surface);
  color: var(--text);
}
.block.on {
  background: var(--surface);
  color: var(--text-2);
}
h1 {
  margin: 0;
  font-size: 24px;
  line-height: 1.3;
  word-break: break-word;
}
.chip.topic {
  background: color-mix(in srgb, var(--link) 12%, transparent);
  color: var(--link);
}
.top-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  margin: 8px 0 0;
  color: var(--link);
  font-size: 13px;
}
.home {
  color: var(--link);
  font-size: 14px;
}
.refresh {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: 10px;
  padding: 2px 10px;
  border: 1px solid var(--line);
  border-radius: 999px;
  color: var(--text-2);
  font-size: 12px;
  vertical-align: 1px;
}
.refresh:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent);
}
.refresh .spin {
  animation: spin 0.8s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.refresh-note {
  margin-left: 8px;
  color: var(--text-3);
  font-size: 12px;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 10px 0 0;
}
.chip {
  padding: 2px 10px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 12px;
}
.desc {
  margin: 6px 0 0;
  color: var(--text-2);
  font-size: 14px;
}
.about {
  max-width: 880px;
  margin: -12px auto 28px;
  padding: 14px 18px;
  border-radius: 14px;
  background: var(--surface);
}
.about h2 {
  margin: 0 0 4px;
  font-size: 14px;
}
.about p {
  margin: 0;
  color: var(--text-2);
  font-size: 14px;
  line-height: 1.7;
  /* An author's own text keeps its line breaks. */
  white-space: pre-line;
}
.about-html {
  color: var(--text-2);
  font-size: 14px;
  line-height: 1.75;
  word-break: break-word;
}
.about-html.clamped {
  max-height: 16em;
  overflow: hidden;
  mask-image: linear-gradient(to bottom, #000 70%, transparent);
}
.about-html :deep(h1),
.about-html :deep(h2),
.about-html :deep(h3) {
  margin: 14px 0 6px;
  color: var(--text);
  font-size: 15px;
}
.about-html :deep(p),
.about-html :deep(ul),
.about-html :deep(ol) {
  margin: 6px 0;
}
.about-html :deep(img) {
  height: auto;
  border-radius: 8px;
}
.about-html :deep(table) {
  display: block;
  max-width: 100%;
  overflow-x: auto;
  border-collapse: collapse;
}
.about-html :deep(td),
.about-html :deep(th) {
  padding: 4px 8px;
  border: 1px solid var(--line);
}
.about .more {
  margin-top: 6px;
  color: var(--link);
  font-size: 13px;
}
.about a {
  display: inline-block;
  margin-top: 6px;
  color: var(--link);
  font-size: 13px;
}
.chip.link:hover {
  color: var(--text);
}
.chip.source {
  background: var(--accent-soft);
  color: var(--accent);
}
.stats {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  margin: 10px 0 0;
  color: var(--text-2);
  font-size: 13px;
}
.stats b {
  color: var(--text);
}
.stats .mine {
  color: var(--text-3);
}
.stats .mine:hover {
  color: var(--link);
}
.missing {
  margin: 48px 0;
  text-align: center;
  color: var(--text-2);
}
@media (max-width: 639px) {
  .profile {
    gap: 16px;
    margin: 16px 6px 20px;
  }
  .profile :deep(.avatar) {
    width: 64px !important;
    height: 64px !important;
  }
  h1 {
    font-size: 20px;
  }
}
</style>
