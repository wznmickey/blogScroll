<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api, cachedPost, categoryName, rememberPosts } from '../api'
import { options } from '../options'
import { direction, sequence } from '../postNav'
import { postClosed, postOpened, track } from '../telemetry'
import { blogPath, fullDate, hostOf, hue, initial } from '../format'
import { recordVisit } from '../library'
import { openInReader, readerIntent, readerIntentEnd, readerOpen, readerShare, warmUp } from '../reader'
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
  heat: '其他读者在看',
} as const
const why = computed(() => {
  const w = card.value?.why
  if (!w) return null
  const parts = (Object.keys(WHY_LABELS) as (keyof typeof WHY_LABELS)[])
    .map((k) => ({ label: WHY_LABELS[k], days: Math.round((w[k] ?? 0) * 10) / 10 }))
    .filter((p) => p.days !== 0)
  const sum = parts.reduce((s, p) => s + p.days, 0)
  return { age: w.age_days, parts, effective: Math.round((w.age_days - sum) * 10) / 10 }
})
const days = (d: number) => `${d > 0 ? '提前' : '推后'} ${Math.abs(d)} 天`

const route = useRoute()
const router = useRouter()
const related = ref<PostCard[]>([])
/** 热度 out of 100, when shown (settings). */
const heat = computed(() => (options.value.showHeat && post.value?.heat ? Math.round(post.value.heat * 100) : null))

// ---- The next and previous post: wheel, swipe, ↑ ↓ (postNav.ts) ----

/** How it came in: from below (next), from above (previous), or opened. */
const entrance = direction.value
direction.value = 0
const neighbor = (step: 1 | -1) => {
  const list = sequence.value?.items() ?? []
  const i = list.findIndex((p) => p.id === props.id)
  return i < 0 ? undefined : list[i + step]
}
const next = ref<PostCard | undefined>(neighbor(1))
/** Opened from a list (not a search result, say): there is a way on. */
const inSequence = !!sequence.value?.items().some((p) => p.id === props.id)
const edge = ref<'first' | 'last' | null>(null)
let moving = false
async function go(step: 1 | -1) {
  if (moving || !sequence.value || !inSequence) return
  let to = neighbor(step)
  if (!to && step === 1 && sequence.value.more) {
    moving = true
    await sequence.value.more()
    moving = false
    to = neighbor(1)
  }
  if (!to) {
    edge.value = step === 1 ? 'last' : 'first'
    setTimeout(() => (edge.value = null), 1500)
    return
  }
  moving = true
  direction.value = step
  rememberPosts([to])
  router.replace({ path: route.path, query: { ...route.query, p: String(to.id) } })
}

// Wheel: where the text has no more to scroll that way, a firm turn moves on
// (once: a trackpad keeps sending after the fingers stop).
const scrollEl = ref<HTMLElement>()
let wheelSum = 0
let wheelIdle = 0
let wheelQuietUntil = 0
function onWheel(e: WheelEvent) {
  if (Math.abs(e.deltaY) < Math.abs(e.deltaX)) return
  const down = e.deltaY > 0
  const el = scrollEl.value
  const inText = !!el && el.contains(e.target as Node)
  if (inText && el && (down ? el.scrollTop + el.clientHeight < el.scrollHeight - 2 : el.scrollTop > 0)) {
    wheelSum = 0
    return
  }
  e.preventDefault()
  if (Date.now() < wheelQuietUntil) return
  wheelSum += e.deltaY
  clearTimeout(wheelIdle)
  wheelIdle = window.setTimeout(() => (wheelSum = 0), 250)
  if (Math.abs(wheelSum) >= (inText ? 220 : 90)) {
    wheelSum = 0
    wheelQuietUntil = Date.now() + 900
    void go(down ? 1 : -1)
  }
}

// Swipe (phones: the whole sheet scrolls): past its end, up for the next; past its top, down for the previous.
let touch: { y: number; atEnd: boolean; atTop: boolean } | null = null
function onTouchStart(e: TouchEvent) {
  const s = e.currentTarget as HTMLElement
  const el = s.scrollHeight > s.clientHeight + 2 ? s : scrollEl.value
  if (!el) return
  touch = {
    y: e.touches[0].clientY,
    atEnd: el.scrollTop + el.clientHeight >= el.scrollHeight - 2,
    atTop: el.scrollTop <= 0,
  }
}
function onTouchEnd(e: TouchEvent) {
  if (!touch) return
  const dy = touch.y - (e.changedTouches[0]?.clientY ?? touch.y)
  if (dy > 80 && touch.atEnd) void go(1)
  else if (dy < -80 && touch.atTop) void go(-1)
  touch = null
}
const brokenThumbs = reactive(new Set<number>())
const relatedLink = (id: number) => ({ path: route.path, query: { ...route.query, p: String(id) } })

onMounted(async () => {
  document.addEventListener('keydown', onKey)
  window.addEventListener('resize', placeSheet)
  if (box.value) resized.observe(box.value)
  placeSheet()
  document.documentElement.style.overflow = 'hidden'
  if (card.value) recordVisit(card.value)
  if (card.value) postOpened(card.value.url)
  api
    .related(props.id, 6)
    .then((r) => {
      rememberPosts(r)
      related.value = r
    })
    .catch(() => {})
  try {
    detail.value = await api.post(props.id)
    if (!card.value) {
      recordVisit(detail.value)
      postOpened(detail.value.url)
    }
  } catch {
    error.value = true
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('keydown', onKey)
  window.removeEventListener('resize', placeSheet)
  resized.disconnect()
  document.documentElement.style.overflow = ''
  postClosed()
})

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') return emit('close')
  if ((e.target as HTMLElement)?.closest?.('input, textarea, select, [contenteditable]')) return
  if (e.key === 'ArrowDown' || e.key === 'j') {
    e.preventDefault()
    void go(1)
  } else if (e.key === 'ArrowUp' || e.key === 'k') {
    e.preventDefault()
    void go(-1)
  }
}

// On a wide screen the original opens beside the feed, and this moves aside.
function openBeside(e: MouseEvent, url: string, title: string) {
  const p = post.value
  if (p && url === p.url) track('read', p.url)
  if (p && openInReader({ url, title, blog: p.blog, post: p }, e)) e.preventDefault()
}

// While the reader pane is open, the sheet keeps its layout and slides left,
// into the room the pane leaves (like the page behind it, see placePage),
// smaller as well when it would not fit there. Where it would have to shrink
// below SQUEEZE, it keeps only its text column instead (the picture goes to
// the top of the text), which fits: the pane never covers the text or its
// buttons. `box` is its place without the move, to measure; the sheet itself
// is moved (translate / scale, beside its opening animation's transform).
const GAP = 16
const SQUEEZE = 0.8
const box = ref<HTMLElement>()
const aside = ref({ x: 0, scale: 1 })
const narrow = ref(false)
const resized = new ResizeObserver(() => placeSheet())
function placeSheet() {
  const paneW = readerOpen.value ? readerShare.value * window.innerWidth : 0
  const el = box.value
  const r = el?.getBoundingClientRect()
  if (!el || !r || !paneW || !r.width) {
    narrow.value = false
    aside.value = { x: 0, scale: 1 }
    return
  }
  const zoom = parseFloat(getComputedStyle(el).zoom) || 1
  const room = document.documentElement.clientWidth - paneW
  // The whole sheet's width, as its styles give it (below).
  const whole = Math.min((hasCover.value ? 1120 : 1000) * zoom, window.innerWidth - 48)
  const squeezed = (room - 2 * GAP) / whole < SQUEEZE
  if (squeezed !== narrow.value) {
    // Its width changes: placed again once it has (ResizeObserver).
    narrow.value = squeezed
    return
  }
  const scale = Math.min(1, (room - 2 * GAP) / r.width)
  const left = Math.max(GAP, (room - r.width * scale) / 2)
  // Scaled about its centre, its left edge moves in by half what it loses;
  // the move is in the box's zoomed pixels (text size).
  aside.value = { x: (left - r.left - (r.width * (1 - scale)) / 2) / zoom, scale }
}
watch([readerOpen, readerShare], placeSheet)
const asideStyle = computed(() =>
  aside.value.x || aside.value.scale !== 1
    ? { translate: `${aside.value.x.toFixed(1)}px 0`, scale: aside.value.scale.toFixed(3) }
    : undefined,
)

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
  <div class="overlay" :class="{ moved: entrance !== 0 }" @click.self="emit('close')">
    <div ref="box" class="box" :class="{ 'no-media': !hasCover, narrow }" @click.self="emit('close')">
      <div
        class="sheet"
        :class="{ 'no-media': !hasCover, 'from-below': entrance === 1, 'from-above': entrance === -1 }"
        @wheel="onWheel"
        @touchstart.passive="onTouchStart"
        @touchend="onTouchEnd"
        :style="asideStyle"
        role="dialog"
        aria-modal="true"
        :aria-label="post?.title"
      >
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
            <RouterLink :to="blogPath(post.blog)" class="who">
              <BlogAvatar :name="post.blog.name" :icon="post.blog.icon" :size="30" />
              <span>{{ post.blog.name }}</span>
            </RouterLink>
            <FollowButton :blog-id="post.blog.id" :blog-key="post.blog.key" small />
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
              <RouterLink :to="blogPath(post.blog)" class="who">
                <BlogAvatar :name="post.blog.name" :icon="post.blog.icon" :size="36" />
                <span>{{ post.blog.name }}</span>
              </RouterLink>
              <FollowButton :blog-id="post.blog.id" :blog-key="post.blog.key" />
            </header>

            <div ref="scrollEl" class="scroll">
              <img
                v-if="narrow && hasCover"
                :src="post.cover!"
                alt=""
                referrerpolicy="no-referrer"
                class="banner"
                @error="coverFailed = true"
              />
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
                <span v-if="heat" class="heat" :title="`比 ${heat}% 的文章更受读者关注（最近 30 天）`"> · 热度 {{ heat }}</span>
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

              <!-- What comes next where the post came from: scroll on, or click. -->
              <button v-if="next" type="button" class="next" @click="go(1)">
                <span class="next-hint">继续往下：下一篇</span>
                <span class="next-title">{{ next.title }}</span>
                <span class="next-blog">{{ next.blog.name }}</span>
              </button>
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
        <div v-if="inSequence" class="steps" aria-label="上一篇 / 下一篇">
          <button type="button" title="上一篇（↑ 或 K）" aria-label="上一篇" @click="go(-1)">
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="m6 15 6-6 6 6" /></svg>
          </button>
          <button type="button" title="下一篇（↓ 或 J）" aria-label="下一篇" @click="go(1)">
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="m6 9 6 6 6-6" /></svg>
          </button>
        </div>
        <p v-if="edge" class="edge">{{ edge === 'last' ? '已经是最后一篇了' : '已经是第一篇了' }}</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  /* Under the reader pane (ReaderPane.vue), which takes the right part. */
  z-index: 50;
  display: grid;
  place-items: center;
  padding: 24px;
  background: var(--overlay);
  animation: fade 0.18s ease-out;
}
.box {
  /* Text size (settings). Lengths in here are zoomed with it, so the room
     the window leaves (less the overlay's padding) is divided back: the
     sheet never gets wider or taller than the window, whatever the size. */
  zoom: var(--zoom, 1);
  width: min(1120px, (100vw - 48px) / var(--zoom, 1));
  height: min(860px, (100vh - 48px) / var(--zoom, 1));
  height: min(860px, (100dvh - 48px) / var(--zoom, 1));
}
.box.no-media {
  width: min(1000px, (100vw - 48px) / var(--zoom, 1));
}
/* Beside the reader pane in a narrow room: the text column alone. */
.box.narrow {
  width: min(440px, (100vw - 48px) / var(--zoom, 1));
}
.narrow .media {
  display: none;
}
.narrow .panel {
  flex: 1 1 auto;
  border-left: 0;
}
/* Clear of the close button. */
.narrow .panel-head {
  padding-left: 62px;
}
.banner {
  display: block;
  width: 100%;
  max-height: 260px;
  margin-bottom: 14px;
  border-radius: 12px;
  object-fit: cover;
  background: var(--surface);
}
.sheet {
  position: relative;
  display: flex;
  width: 100%;
  height: 100%;
  overflow: hidden;
  border-radius: 20px;
  background: var(--bg);
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
  animation: pop 0.22s cubic-bezier(0.2, 0.9, 0.3, 1.1);
  /* Moving aside for the reader pane, as fast as the pane opens. */
  transition:
    translate 0.3s cubic-bezier(0.2, 0.8, 0.2, 1),
    scale 0.3s cubic-bezier(0.2, 0.8, 0.2, 1);
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
/* Moved to the next / previous post: no new dimming, the sheet comes in from that side. */
.overlay.moved {
  animation: none;
}
.sheet.from-below {
  animation: from-below 0.28s cubic-bezier(0.2, 0.8, 0.2, 1);
}
.sheet.from-above {
  animation: from-above 0.28s cubic-bezier(0.2, 0.8, 0.2, 1);
}
@keyframes from-below {
  from {
    opacity: 0.4;
    transform: translateY(40%);
  }
}
@keyframes from-above {
  from {
    opacity: 0.4;
    transform: translateY(-40%);
  }
}
.steps {
  position: absolute;
  top: 50%;
  /* At the picture's right edge, beside the text column. */
  right: calc(420px + 12px);
  z-index: 3;
  display: flex;
  flex-direction: column;
  gap: 8px;
  transform: translateY(-50%);
}
.steps button {
  display: grid;
  place-items: center;
  width: 34px;
  height: 34px;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.35);
  color: #fff;
  opacity: 0.6;
  transition: opacity 0.15s;
}
.steps button:hover {
  opacity: 1;
}
.narrow .steps {
  display: none;
}
.edge {
  position: absolute;
  bottom: 84px;
  left: 50%;
  z-index: 3;
  margin: 0;
  padding: 6px 14px;
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.7);
  color: #fff;
  font-size: 13px;
  transform: translateX(-50%);
}
.next {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
  margin: 24px 0 4px;
  padding: 12px 14px;
  border: 1px dashed var(--line);
  border-radius: 12px;
  text-align: left;
}
.next:hover {
  border-color: var(--accent);
}
.next-hint {
  color: var(--text-3);
  font-size: 12px;
}
.next-title {
  font-weight: 600;
  line-height: 1.4;
}
.next-blog {
  color: var(--text-3);
  font-size: 12px;
}
.heat {
  color: #f97316;
}
@media (prefers-reduced-motion: reduce) {
  .sheet,
  .sheet.from-below,
  .sheet.from-above {
    transition: none;
    animation: none;
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
  .box,
  .box.no-media {
    width: 100%;
    height: 100%;
  }
  .sheet {
    flex-direction: column;
    border-radius: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    box-shadow: none;
    animation: slide 0.22s ease-out;
  }
  .close,
  .panel-head,
  .steps {
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
