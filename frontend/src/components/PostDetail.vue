<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { api, cachedPost, categoryName, rememberPosts } from '../api'
import { fullDate, hostOf, hue, initial } from '../format'
import { recordVisit } from '../library'
import { openInReader, readerIntent, readerIntentEnd, warmUp } from '../reader'
import type { PostCard, PostDetail } from '../types'
import BlogAboutCard from './BlogAboutCard.vue'
import BlogAvatar from './BlogAvatar.vue'
import CommentEmbed from './CommentEmbed.vue'
import FollowButton from './FollowButton.vue'
import LibraryButton from './LibraryButton.vue'
import TextCover from './TextCover.vue'

const props = defineProps<{ id: number }>()
const emit = defineEmits<{ close: [] }>()

const card = ref<PostCard | undefined>(cachedPost(props.id))
const detail = ref<PostDetail | null>(null)
const error = ref(false)
const coverFailed = ref(false)
const post = computed<PostCard | PostDetail | undefined>(() => detail.value ?? card.value)
const hasCover = computed(() => !!post.value?.cover && !coverFailed.value)
const comments = computed(() => detail.value?.comments ?? null)
const commentsLink = computed(() => post.value?.comments_url ?? post.value?.url)
const ownTags = computed(() => post.value?.tags ?? [])
const autoTags = computed(() => post.value?.auto_tags ?? [])
const category = computed(() => (post.value?.category && post.value.category.id !== 'other' ? post.value.category : null))

// Opened from 推荐 / 反推荐: what put it where it was (the card keeps it).
const WHY_LABELS = {
  noise: '随机打散',
  same_blog: '同一博主的第几篇',
  interest: '和你的兴趣相近',
  category: '你常看的分类',
  blog: '你关注 / 读过的博主',
  seen: '已经看过',
  shown: '刷到过没点开',
} as const
const why = computed(() => {
  const w = card.value?.why
  if (!w) return null
  const parts = (Object.keys(WHY_LABELS) as (keyof typeof WHY_LABELS)[])
    .map((k) => ({ label: WHY_LABELS[k], days: Math.round(w[k] * 10) / 10 }))
    .filter((p) => p.days !== 0)
  const sum = parts.reduce((s, p) => s + p.days, 0)
  return { age: w.age_days, parts, effective: Math.round((w.age_days - sum) * 10) / 10 }
})
const days = (d: number) => `${d > 0 ? '提前' : '推后'} ${Math.abs(d)} 天`

const route = useRoute()
const related = ref<PostCard[]>([])
const brokenThumbs = reactive(new Set<number>())
const relatedLink = (id: number) => ({ path: route.path, query: { ...route.query, p: String(id) } })

onMounted(async () => {
  document.addEventListener('keydown', onKey)
  document.documentElement.style.overflow = 'hidden'
  if (card.value) recordVisit(card.value)
  api
    .related(props.id, 6)
    .then((r) => {
      rememberPosts(r)
      related.value = r
    })
    .catch(() => {})
  try {
    detail.value = await api.post(props.id)
    if (!card.value) recordVisit(detail.value)
  } catch {
    error.value = true
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('keydown', onKey)
  document.documentElement.style.overflow = ''
})

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}

// On a wide screen the original opens beside the feed, and this closes.
function openBeside(e: MouseEvent, url: string, title: string) {
  const p = post.value
  if (p && openInReader({ url, title, blog: p.blog, post: p }, e)) {
    e.preventDefault()
    emit('close')
  }
}

// The original may be next: be connected to its site, and once the pointer
// rests on a link to it, load it in the (hidden) pane.
watch(
  () => post.value?.url,
  (url) => url && warmUp(url),
  { immediate: true },
)
function ahead(url: string, title: string, delay?: number) {
  const p = post.value
  if (p) readerIntent({ url, title, blog: p.blog, post: p }, delay)
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="sheet" :class="{ 'no-media': !hasCover }" role="dialog" aria-modal="true" :aria-label="post?.title">
      <button class="close" aria-label="关闭" @click="emit('close')">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M6 6l12 12M18 6 6 18" />
        </svg>
      </button>

      <template v-if="post">
        <header class="mobile-bar">
          <button class="back" aria-label="返回" @click="emit('close')">
            <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <path d="m15 5-7 7 7 7" />
            </svg>
          </button>
          <RouterLink :to="`/blog/${post.blog.id}`" class="who">
            <BlogAvatar :name="post.blog.name" :icon="post.blog.icon" :size="30" />
            <span>{{ post.blog.name }}</span>
          </RouterLink>
          <FollowButton :blog-id="post.blog.id" small />
        </header>

        <div class="media">
          <img
            v-if="hasCover"
            :src="post.cover!"
            alt=""
            referrerpolicy="no-referrer"
            @error="coverFailed = true"
          />
          <TextCover v-else class="media-text" :post="post" :ratio="1.33" large />
        </div>

        <section class="panel">
          <header class="panel-head">
            <RouterLink :to="`/blog/${post.blog.id}`" class="who">
              <BlogAvatar :name="post.blog.name" :icon="post.blog.icon" :size="36" />
              <span>{{ post.blog.name }}</span>
            </RouterLink>
            <FollowButton :blog-id="post.blog.id" />
          </header>

          <div class="scroll">
            <h1 class="title">{{ post.title }}</h1>
            <div v-if="detail?.content_html" class="content" v-html="detail.content_html" />
            <template v-else>
              <!-- Its formatting, when the feed had some: a few tags without attributes, written on the server. -->
              <div v-if="post.excerpt_html" class="excerpt rich" v-html="post.excerpt_html" />
              <p v-else-if="post.excerpt" class="excerpt">{{ post.excerpt }}</p>
              <p class="note">
                以上为 RSS 摘要。为尊重作者，完整内容请到原博客阅读。
              </p>
            </template>
            <details v-if="why" class="why">
              <summary>为什么推荐这篇</summary>
              <ul>
                <li><span>发布于 {{ why.age }} 天前</span><span class="w">起点</span></li>
                <li v-for="p in why.parts" :key="p.label">
                  <span>{{ p.label }}</span><span class="w" :class="{ up: p.days > 0 }">{{ days(p.days) }}</span>
                </li>
              </ul>
              <p>
                排序时相当于 {{ why.effective >= 0 ? `${why.effective} 天前` : `${-why.effective} 天后` }}发布的文章。
                <RouterLink to="/settings">调整推荐权重</RouterLink>
              </p>
            </details>
            <p v-if="category" class="category">
              <span class="label">分类</span>
              <RouterLink :to="{ path: '/', query: { c: `cat:${category.id}` } }">{{ categoryName(category) }}</RouterLink>
            </p>
            <p v-if="ownTags.length || autoTags.length" class="tags">
              <RouterLink v-for="t in ownTags" :key="t" :to="{ path: '/', query: { c: `tag:${t}` } }">#{{ t }}</RouterLink>
              <RouterLink
                v-for="t in autoTags"
                :key="t"
                :to="{ path: '/', query: { c: `tag:${t}` } }"
                class="auto"
                title="根据摘要自动提取的标签"
                >#{{ t }}</RouterLink
              >
            </p>
            <p class="date">
              {{ post.undated ? '发布日期未知' : fullDate(post.published_at ?? post.sort_at) }}
              <template v-if="post.author && post.author !== post.blog.name"> · {{ post.author }}</template>
            </p>
            <p v-if="error" class="note">详情加载失败，可直接阅读原文。</p>

            <BlogAboutCard :blog="post.blog" />

            <section v-if="post.comment_count != null || comments" class="comments">
              <header class="comments-head">
                <span class="comments-title">评论</span>
                <span v-if="post.comment_count != null" class="comments-count">{{ post.comment_count }}</span>
                <span v-if="comments" class="comments-sys">{{ comments.system }}</span>
                <a
                  :href="commentsLink"
                  target="_blank"
                  rel="noopener"
                  class="comments-go"
                  @pointerenter="ahead(commentsLink!, `评论 · ${post.title}`)"
                  @pointerleave="readerIntentEnd"
                  @focus="ahead(commentsLink!, `评论 · ${post.title}`)"
                  @pointerdown="ahead(commentsLink!, `评论 · ${post.title}`, 0)"
                  @click="openBeside($event, commentsLink!, `评论 · ${post.title}`)"
                  >去原文评论 ↗</a
                >
              </header>
              <CommentEmbed v-if="comments?.embed" :key="post.id" :config="comments.embed" />
            </section>

            <section v-if="related.length" class="related">
              <h2>相似文章</h2>
              <ul>
                <li v-for="r in related" :key="r.id">
                  <RouterLink :to="relatedLink(r.id)" replace class="rel">
                    <img
                      v-if="r.cover && !brokenThumbs.has(r.id)"
                      :src="r.cover"
                      alt=""
                      loading="lazy"
                      referrerpolicy="no-referrer"
                      class="thumb"
                      @error="brokenThumbs.add(r.id)"
                    />
                    <span v-else class="thumb thumb-text" :style="{ '--h': hue(r.blog.name) }">{{
                      initial(r.title)
                    }}</span>
                    <span class="rel-text">
                      <span class="rel-title">{{ r.title }}</span>
                      <span class="rel-blog">{{ r.blog.name }}</span>
                    </span>
                  </RouterLink>
                </li>
              </ul>
            </section>
          </div>

          <footer class="actions">
            <LibraryButton :post="post" list="likes" :size="22" label class="act" />
            <LibraryButton :post="post" list="saves" :size="22" label class="act" />
            <span class="host">{{ hostOf(post.url) }}</span>
            <a
              :href="post.url"
              target="_blank"
              rel="noopener"
              class="btn btn-primary read"
              :title="post.blog.framable === false ? '这个网站不允许被嵌入别的页面，会在新标签页打开' : undefined"
              @pointerenter="ahead(post.url, post.title)"
              @pointerleave="readerIntentEnd"
              @focus="ahead(post.url, post.title)"
              @pointerdown="ahead(post.url, post.title, 0)"
              @click="openBeside($event, post.url, post.title)"
              >{{ post.blog.framable === false ? '阅读原文（新标签页）' : '阅读原文' }} ↗</a
            >
          </footer>
        </section>
      </template>
      <div v-else-if="error" class="missing">
        <p>找不到这篇文章</p>
        <button class="btn btn-ghost" @click="emit('close')">返回</button>
      </div>
      <div v-else class="missing"><span class="spinner" /></div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  /* Beside the reader pane, not over it. */
  right: var(--pane-w, 0px);
  z-index: 50;
  display: grid;
  place-items: center;
  padding: 24px;
  background: var(--overlay);
  animation: fade 0.18s ease-out;
}
.sheet {
  /* Text size (settings). */
  zoom: var(--zoom, 1);
  position: relative;
  display: flex;
  width: min(1120px, 100%);
  height: min(860px, 100%);
  overflow: hidden;
  border-radius: 20px;
  background: var(--bg);
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
  animation: pop 0.22s cubic-bezier(0.2, 0.9, 0.3, 1.1);
}
.sheet.no-media {
  width: min(1000px, 100%);
}
.close {
  position: absolute;
  top: 14px;
  left: 14px;
  z-index: 2;
  display: grid;
  place-items: center;
  width: 36px;
  height: 36px;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.45);
  color: #fff;
}
.mobile-bar {
  display: none;
}
.media {
  flex: 1 1 58%;
  min-width: 0;
  display: grid;
  place-items: center;
  background: var(--surface);
}
.media img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.media-text {
  width: 100%;
  height: 100%;
  border-radius: 0;
}
.panel {
  flex: 0 0 420px;
  display: flex;
  flex-direction: column;
  min-width: 0;
  border-left: 1px solid var(--line);
}
.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 20px 24px;
  border-bottom: 1px solid var(--line);
}
.who {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  font-weight: 600;
}
.who span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.scroll {
  flex: 1;
  overflow-y: auto;
  padding: 20px 24px 24px;
  overscroll-behavior: contain;
}
.title {
  margin: 0 0 12px;
  font-size: 19px;
  line-height: 1.45;
  font-weight: 700;
  word-break: break-word;
}
.excerpt {
  margin: 0 0 12px;
  white-space: pre-line;
  word-break: break-word;
}
.excerpt.rich {
  white-space: normal;
  line-height: 1.75;
}
.rich :deep(p),
.rich :deep(ul),
.rich :deep(ol),
.rich :deep(blockquote) {
  margin: 0 0 8px;
}
.rich :deep(ul),
.rich :deep(ol) {
  padding-left: 1.4em;
}
.rich :deep(h4) {
  margin: 12px 0 4px;
  font-size: 15px;
}
.rich :deep(h4:first-child) {
  margin-top: 0;
}
.rich :deep(blockquote) {
  padding-left: 12px;
  border-left: 3px solid var(--line);
  color: var(--text-2);
}
.rich :deep(code) {
  padding: 1px 5px;
  border-radius: 4px;
  background: var(--surface);
  font-size: 0.9em;
}
.why {
  margin: 12px 0;
  padding: 8px 12px;
  border-radius: 10px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
}
.why summary {
  cursor: pointer;
  color: var(--text);
}
.why ul {
  margin: 8px 0 4px;
  padding: 0;
  list-style: none;
}
.why li {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 2px 0;
}
.why .w {
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
.why .w.up {
  color: var(--accent);
}
.why p {
  margin: 4px 0 0;
}
.why a {
  color: var(--link);
}
.note {
  margin: 12px 0;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 4px 10px;
  margin: 12px 0 0;
  color: var(--link);
  font-size: 14px;
}
.category {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin: 12px 0 0;
  font-size: 13px;
}
.category .label {
  color: var(--text-3);
}
.category a {
  padding: 0 10px;
  border-radius: 999px;
  background: var(--accent-soft);
  color: var(--accent);
  line-height: 24px;
}
.tags .auto {
  opacity: 0.7;
}
.related {
  margin-top: 20px;
  padding-top: 16px;
  border-top: 1px solid var(--line);
}
.related h2 {
  margin: 0 0 10px;
  font-size: 15px;
}
.related ul {
  display: grid;
  gap: 10px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.rel {
  display: flex;
  gap: 10px;
  align-items: center;
}
.thumb {
  flex: none;
  width: 64px;
  height: 64px;
  border-radius: 8px;
  object-fit: cover;
  background: var(--surface);
}
.thumb-text {
  display: grid;
  place-items: center;
  background: hsl(var(--h) 60% 88%);
  color: hsl(var(--h) 45% 35%);
  font-size: 22px;
  font-weight: 700;
}
.rel-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.rel-title {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
  font-size: 14px;
  line-height: 1.45;
}
.rel:hover .rel-title {
  color: var(--link);
}
.rel-blog {
  color: var(--text-3);
  font-size: 12px;
}
.date {
  margin: 12px 0 0;
  color: var(--text-3);
  font-size: 13px;
}
.comments {
  margin-top: 20px;
  padding-top: 16px;
  border-top: 1px solid var(--line);
}
.comments-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.comments-title {
  font-weight: 600;
}
.comments-count {
  margin-left: -4px;
  color: var(--text-2);
  font-weight: 600;
}
.comments-sys {
  padding: 0 8px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-3);
  font-size: 12px;
}
.comments-go {
  margin-left: auto;
  color: var(--link);
  font-size: 13px;
}
.actions {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 14px 24px calc(14px + var(--safe-b));
  border-top: 1px solid var(--line);
}
.host {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-align: right;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-3);
  font-size: 13px;
}
.act {
  height: 40px;
}
.read {
  height: 40px;
  padding: 0 22px;
}
.missing {
  display: grid;
  place-items: center;
  gap: 12px;
  width: 100%;
  color: var(--text-2);
}
.spinner {
  width: 24px;
  height: 24px;
  border: 2px solid var(--line);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
.content {
  word-break: break-word;
  line-height: 1.75;
}
.content :deep(img) {
  height: auto;
  border-radius: 8px;
  margin: 8px 0;
}
.content :deep(pre) {
  overflow-x: auto;
  padding: 12px;
  border-radius: 8px;
  background: var(--surface);
  font-size: 13px;
}
.content :deep(a) {
  color: var(--link);
}
.content :deep(blockquote) {
  margin: 8px 0;
  padding-left: 12px;
  border-left: 3px solid var(--line);
  color: var(--text-2);
}

@keyframes fade {
  from {
    opacity: 0;
  }
}
@keyframes pop {
  from {
    opacity: 0;
    transform: scale(0.96);
  }
}
@keyframes slide {
  from {
    transform: translateY(24px);
    opacity: 0;
  }
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* Text-only posts on desktop: just the text, no empty media pane. */
@media (min-width: 768px) {
  .no-media .media {
    flex-basis: 45%;
  }
}

/* Phones and small tablets: full-screen page instead of a modal. */
@media (max-width: 767px) {
  .overlay {
    display: block;
    padding: 0;
    background: var(--bg);
    animation: none;
  }
  .sheet,
  .sheet.no-media {
    flex-direction: column;
    width: 100%;
    height: 100%;
    border-radius: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    box-shadow: none;
    animation: slide 0.22s ease-out;
  }
  .close,
  .panel-head {
    display: none;
  }
  .mobile-bar {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
    height: 52px;
    padding: 0 12px 0 4px;
    background: var(--bg);
    border-bottom: 1px solid var(--line);
  }
  .back {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
  }
  .mobile-bar .who {
    flex: 1;
  }
  .media {
    flex: none;
    max-height: 72vh;
  }
  .media img {
    height: auto;
    max-height: 72vh;
  }
  .media-text {
    min-height: 0;
    aspect-ratio: 4 / 3;
    padding: 40px 28px;
  }
  .media-text :deep(p) {
    font-size: 18px;
  }
  .panel {
    flex: 1 0 auto;
    border-left: 0;
  }
  .scroll {
    overflow: visible;
    padding: 16px 16px 24px;
  }
  .actions {
    position: sticky;
    bottom: 0;
    background: var(--bg);
    padding: 10px 16px calc(10px + var(--safe-b));
  }
  .read {
    flex: 1;
    max-width: 220px;
    margin-left: auto;
  }
  .host {
    display: none;
  }
  .act :deep(span) {
    display: none;
  }
}
</style>
