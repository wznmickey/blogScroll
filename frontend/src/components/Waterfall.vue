<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch, type ComponentPublicInstance } from 'vue'
import { meta, rememberPosts } from '../api'
import { prefs } from '../channels'
import { COVER_DEFAULT, textCoverRatio } from '../format'
import type { Page, PostCard } from '../types'
import FeedCard from './FeedCard.vue'

const props = defineProps<{
  load: (cursor: string | null) => Promise<Page<PostCard>>
  /** Changing this clears the feed and loads from the start. */
  resetKey: string
  /** Cards to mark as new. */
  isFresh?: (post: PostCard) => boolean
}>()
/** A card stayed on screen: at least half of it, for IMPRESSION_MS. Once per card per feed. */
const emit = defineEmits<{ impression: [post: PostCard] }>()

const root = ref<HTMLElement>()
const sentinel = ref<HTMLElement>()
const colEls: HTMLElement[] = []
const columns = shallowRef<PostCard[][]>([])
const loading = ref(false)
const done = ref(false)
const error = ref(false)
const empty = ref(false)

let items: PostCard[] = []
let seen = new Set<number>()
let cursor: string | null = null
let generation = 0
let width = 0
/** Cover height/width once each image has loaded; improves later placement. */
const aspects = new Map<number, number>()

const PRELOAD_PX = 1200
const IMPRESSION_MS = 600

/** Narrowest a card gets when the reader asks for many columns. */
const MIN_CARD = 150

/** The reader's choice, as many as fit at MIN_CARD; else two on a phone,
 *  then one more per 320px from 640px wide, up to the site's `[server] columns`. */
function columnCount(w: number) {
  const chosen = prefs.value.columns
  if (chosen) return Math.max(1, Math.min(chosen, Math.floor((w + gap(w)) / (MIN_CARD + gap(w)))))
  const max = meta.value?.columns ?? 6
  return Math.max(1, Math.min(max, w < 640 ? 2 : 3 + Math.floor((w - 640) / 320)))
}
function gap(w: number) {
  return w < 640 ? 6 : 16
}

function estimate(p: PostCard, colWidth: number) {
  const ratio = p.cover ? (aspects.get(p.id) ?? COVER_DEFAULT) : textCoverRatio(p)
  const charsPerLine = Math.max(1, Math.floor((colWidth - 8) / 14))
  const titleLines = p.title.length > charsPerLine ? 2 : 1
  return colWidth * ratio + titleLines * 20 + 48
}

function shortest(heights: number[]) {
  let best = 0
  for (let i = 1; i < heights.length; i++) if (heights[i] < heights[best] - 1) best = i
  return best
}

function place(newItems: PostCard[], fromScratch: boolean) {
  const n = columnCount(width)
  const g = gap(width)
  const colWidth = (width - g * (n - 1)) / n
  const reuse = !fromScratch && columns.value.length === n
  const cols = reuse ? columns.value.map((c) => c.slice()) : Array.from({ length: n }, () => [] as PostCard[])
  // Real rendered heights where we have them, so loaded images are accounted for.
  const heights = cols.map((_, i) => (reuse ? (colEls[i]?.offsetHeight ?? 0) : 0))
  for (const p of fromScratch || !reuse ? items : newItems) {
    const i = shortest(heights)
    cols[i].push(p)
    heights[i] += estimate(p, colWidth) + g
  }
  columns.value = cols
}

// Impressions: which cards actually stayed on screen.
const cardPosts = new WeakMap<Element, PostCard>()
const dwell = new Map<Element, ReturnType<typeof setTimeout>>()
let impressed = new Set<number>()
let impressionObserver: IntersectionObserver | undefined

function onVisibility(entries: IntersectionObserverEntry[]) {
  for (const e of entries) {
    const post = cardPosts.get(e.target)
    if (!post || impressed.has(post.id)) continue
    if (e.isIntersecting) {
      if (!dwell.has(e.target)) {
        dwell.set(
          e.target,
          setTimeout(() => {
            dwell.delete(e.target)
            if (impressed.has(post.id)) return
            impressed.add(post.id)
            emit('impression', post)
          }, IMPRESSION_MS),
        )
      }
    } else {
      clearTimeout(dwell.get(e.target))
      dwell.delete(e.target)
    }
  }
}

function trackCard(card: Element | ComponentPublicInstance | null, post: PostCard) {
  const el = card && '$el' in card ? (card.$el as Element) : card
  if (!el || cardPosts.get(el) === post || impressed.has(post.id)) return
  cardPosts.set(el, post)
  impressionObserver?.observe(el)
}

function resetImpressions() {
  for (const t of dwell.values()) clearTimeout(t)
  dwell.clear()
  impressed = new Set()
  impressionObserver?.disconnect()
}

function reset() {
  generation++
  resetImpressions()
  items = []
  seen = new Set()
  cursor = null
  columns.value = []
  loading.value = false
  done.value = false
  error.value = false
  empty.value = false
}

async function loadMore() {
  if (loading.value || done.value || width === 0) return
  const gen = generation
  loading.value = true
  error.value = false
  try {
    const page = await props.load(cursor)
    if (gen !== generation) return
    const fresh = page.items.filter((p) => !seen.has(p.id))
    for (const p of fresh) seen.add(p.id)
    items.push(...fresh)
    rememberPosts(fresh)
    place(fresh, false)
    cursor = page.next_cursor
    done.value = !page.next_cursor
    empty.value = items.length === 0
  } catch {
    if (gen === generation) error.value = true
  } finally {
    if (gen === generation) loading.value = false
  }
  // Keep filling until the sentinel is pushed below the fold.
  await nextTick()
  if (gen === generation && !error.value && nearBottom()) void loadMore()
}

function nearBottom() {
  const el = sentinel.value
  return !!el && el.getBoundingClientRect().top < window.innerHeight + PRELOAD_PX
}

function setColEl(el: unknown, i: number) {
  if (el instanceof HTMLElement) colEls[i] = el
}

let resizeObserver: ResizeObserver | undefined
let intersectionObserver: IntersectionObserver | undefined

onMounted(() => {
  resizeObserver = new ResizeObserver(([entry]) => {
    const w = Math.floor(entry.contentRect.width)
    // 0 while detached (e.g. cached by <KeepAlive>); keep the current layout.
    if (w === 0 || w === width) return
    const before = columnCount(width)
    width = w
    if (columnCount(w) !== before && items.length) place([], true)
    if (!items.length && !loading.value && !done.value) void loadMore()
  })
  resizeObserver.observe(root.value!)
  intersectionObserver = new IntersectionObserver(
    (entries) => {
      if (entries.some((e) => e.isIntersecting)) void loadMore()
    },
    { rootMargin: `0px 0px ${PRELOAD_PX}px 0px` },
  )
  intersectionObserver.observe(sentinel.value!)
  impressionObserver = new IntersectionObserver(onVisibility, { threshold: 0.5 })
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  intersectionObserver?.disconnect()
  resetImpressions()
})

// The site's setting arrives with its meta, possibly after the first cards;
// the reader's can change at any time.
watch([() => meta.value?.columns, () => prefs.value.columns], () => {
  if (items.length) place([], true)
})

watch(
  () => props.resetKey,
  () => {
    reset()
    void loadMore()
  },
)

function retry() {
  error.value = false
  void loadMore()
}

defineExpose({ refresh: () => (reset(), loadMore()) })
</script>

<template>
  <div ref="root" class="waterfall">
    <div class="columns">
      <div v-for="(col, i) in columns" :key="i" :ref="(el) => setColEl(el, i)" class="column">
        <FeedCard
          v-for="post in col"
          :key="post.id"
          :ref="(c) => trackCard(c as ComponentPublicInstance | null, post)"
          :post="post"
          :fresh="isFresh?.(post)"
          @aspect="(r) => aspects.set(post.id, r)"
        />
      </div>
    </div>

    <div v-if="loading && !columns.some((c) => c.length)" class="columns skeletons" aria-hidden="true">
      <div v-for="i in 4" :key="i" class="column">
        <div v-for="j in 3" :key="j" class="skeleton" :style="{ aspectRatio: `1 / ${[1.3, 0.9, 1.1][(i + j) % 3]}` }" />
      </div>
    </div>

    <div ref="sentinel" class="status">
      <span v-if="loading && columns.some((c) => c.length)" class="spinner" aria-label="加载中" />
      <button v-else-if="error" class="btn btn-ghost" @click="retry">加载失败，点此重试</button>
      <slot v-else-if="empty" name="empty"><p class="hint">这里还没有内容</p></slot>
      <p v-else-if="done" class="hint">— 到底啦 —</p>
    </div>
  </div>
</template>

<style scoped>
.columns {
  display: flex;
  align-items: flex-start;
  gap: 16px;
}
.column {
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.skeletons .column:nth-child(n + 3) {
  display: none;
}
@media (min-width: 640px) {
  .skeletons .column:nth-child(n + 3) {
    display: flex;
  }
}
.skeleton {
  border-radius: var(--radius-cover);
  background: linear-gradient(90deg, var(--surface) 0%, var(--line) 50%, var(--surface) 100%);
  background-size: 200% 100%;
  animation: shimmer 1.4s infinite linear;
}
@keyframes shimmer {
  to {
    background-position: -200% 0;
  }
}
.status {
  display: flex;
  justify-content: center;
  min-height: 80px;
  padding: 24px 0 calc(32px + var(--safe-b));
}
.hint {
  margin: 0;
  color: var(--text-3);
  font-size: 13px;
}
.spinner {
  width: 22px;
  height: 22px;
  border: 2px solid var(--line);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (max-width: 639px) {
  .columns,
  .column {
    gap: 6px;
  }
}
</style>
