<script setup lang="ts">
// The right part of a split page: a blog's own page in a frame, with a bar to
// like or save the post, reload, open it in a new tab, or close. The divider
// can be dragged. It slides in and out; while closed it stays in the page out
// of sight, so a page loading ahead (readerAhead) is the very frame that shows.
import { blogPath } from '../format'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import {
  closeReader,
  markBlocked,
  placePage,
  readerAhead,
  readerOpen,
  readerPage,
  readerShare,
  setReaderShare,
  type ReaderPage,
} from '../reader'
import BlogAvatar from './BlogAvatar.vue'
import LibraryButton from './LibraryButton.vue'

/** As long as the slide out (see the styles). */
const SLIDE_MS = 220

interface Frame {
  id: number
  url: string
  loaded: boolean
}

const route = useRoute()
const resizing = ref(false)
const open = computed(() => readerOpen.value && !!readerPage.value)
/** What the bar shows: the page, also while the pane slides away. */
const shownPage = ref<ReaderPage | null>(null)
// The shown frame and the one loading ahead. Each keeps its element (frames
// are only added at the end and removed), as moving a frame reloads it.
const frames = ref<Frame[]>([])
const currentId = ref<number | null>(null)
const current = computed(() => frames.value.find((f) => f.id === currentId.value))
let seq = 0
let dropTimer = 0

function frameFor(url: string): number {
  const found = frames.value.find((f) => f.url === url)
  if (found) return found.id
  frames.value.push({ id: ++seq, url, loaded: false })
  return seq
}

function prune() {
  const ahead = frames.value.find((f) => f.url === readerAhead.value)?.id
  frames.value = frames.value.filter((f) => f.id === currentId.value || f.id === ahead)
}

// A new page starts from the top, not where the last one was; one loaded
// ahead is already there.
watch(
  [open, () => readerPage.value?.url, readerAhead],
  ([isOpen, url, ahead]) => {
    clearTimeout(dropTimer)
    if (ahead && ahead !== url) frameFor(ahead)
    if (isOpen && url) {
      shownPage.value = readerPage.value
      currentId.value = frameFor(url)
      prune()
    } else {
      // Gone once out of sight, which also stops whatever it was playing.
      dropTimer = window.setTimeout(() => {
        currentId.value = null
        prune()
      }, SLIDE_MS + 50)
    }
  },
  { immediate: true },
)
onBeforeUnmount(() => clearTimeout(dropTimer))

// Another page lays out differently: it goes left by its own margin.
watch(
  () => route.path,
  async () => {
    await nextTick()
    if (open.value) placePage()
  },
)

function reload() {
  const page = readerPage.value
  if (!page) return
  frames.value = frames.value.filter((f) => f.id !== currentId.value)
  currentId.value = frameFor(page.url)
}

function giveUp() {
  const page = readerPage.value
  if (!page) return
  window.open(page.url, '_blank', 'noopener')
  markBlocked(page.blog.id)
  closeReader()
}

function startResize(e: PointerEvent) {
  if (e.button !== 0) return
  resizing.value = true
  ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
}

function moveResize(e: PointerEvent) {
  if (resizing.value) setReaderShare((window.innerWidth - e.clientX) / window.innerWidth, false)
}

function endResize() {
  if (!resizing.value) return
  resizing.value = false
  setReaderShare(readerShare.value, true)
}

// Esc closes the pane when nothing else (an open post, a text field) wants it.
function onKey(e: KeyboardEvent) {
  if (e.key !== 'Escape' || !readerOpen.value || route.query.p) return
  const t = e.target as HTMLElement | null
  if (t?.closest('input, textarea, select, [contenteditable]')) return
  closeReader()
}
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))
</script>

<template>
  <aside
    class="reader"
    :class="{ open }"
    :style="{ width: `${(readerShare * 100).toFixed(2)}vw` }"
    :aria-hidden="!open"
    aria-label="原文"
  >
    <div
      class="handle"
      role="separator"
      aria-orientation="vertical"
      title="拖动调整宽度"
      @pointerdown="startResize"
      @pointermove="moveResize"
      @pointerup="endResize"
      @pointercancel="endResize"
    />
    <header v-if="shownPage" class="bar">
      <RouterLink :to="blogPath(shownPage.blog)" class="who" :title="shownPage.blog.name">
        <BlogAvatar :name="shownPage.blog.name" :icon="shownPage.blog.icon" :size="24" />
      </RouterLink>
      <span class="title" :title="shownPage.title">{{ shownPage.title }}</span>
      <template v-if="shownPage.post">
        <LibraryButton :post="shownPage.post" list="likes" :size="18" class="act" />
        <LibraryButton :post="shownPage.post" list="saves" :size="18" class="act" />
      </template>
      <button class="act" title="重新加载" aria-label="重新加载" @click="reload">
        <svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7" />
        </svg>
      </button>
      <a :href="shownPage.url" target="_blank" rel="noopener" class="act" title="在新标签页打开" aria-label="在新标签页打开">
        <svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" />
        </svg>
      </a>
      <button class="act" title="关闭（Esc）" aria-label="关闭" @click="closeReader">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M6 6l12 12M18 6 6 18" />
        </svg>
      </button>
    </header>
    <p v-if="shownPage && shownPage.blog.framable == null" class="hint">
      页面空白或显示「拒绝连接」？有的网站不允许被嵌入。
      <button class="link" @click="giveUp">改用新标签页打开</button>
    </p>
    <div class="frames">
      <iframe
        v-for="f in frames"
        :key="f.id"
        :src="f.url"
        :title="f.id === currentId ? shownPage?.title : undefined"
        :class="{ shown: f.id === currentId }"
        sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox allow-forms allow-modals"
        allow="fullscreen"
        @load="f.loaded = true"
      />
      <div v-if="open && current && !current.loaded" class="loading" role="progressbar" aria-label="加载中" />
    </div>
    <!-- Keeps the frame from swallowing the pointer while the divider moves. -->
    <div v-if="resizing" class="shield" />
  </aside>
</template>

<style scoped>
.reader {
  position: fixed;
  top: 0;
  right: 0;
  bottom: 0;
  /* Over an open post's overlay (PostDetail.vue), which slides aside for it. */
  z-index: 55;
  display: flex;
  flex-direction: column;
  border-left: 1px solid var(--line);
  background: var(--bg);
  /* Closed: just past the right edge, laid out at its size (so a page
     loading ahead lays out right) but not shown. */
  transform: translateX(100%);
  visibility: hidden;
  transition:
    transform 0.22s cubic-bezier(0.4, 0, 1, 1),
    box-shadow 0.22s,
    visibility 0s 0.22s;
}
.reader.open {
  transform: none;
  visibility: visible;
  box-shadow: -8px 0 28px rgb(0 0 0 / 0.08);
  transition:
    transform 0.3s cubic-bezier(0.2, 0.8, 0.2, 1),
    box-shadow 0.3s,
    visibility 0s;
}
.handle {
  position: absolute;
  top: 0;
  bottom: 0;
  left: -5px;
  z-index: 2;
  width: 10px;
  cursor: col-resize;
}
.handle:hover {
  background: linear-gradient(to right, transparent 4px, var(--accent-soft) 4px, var(--accent-soft) 6px, transparent 6px);
}
.bar {
  display: flex;
  flex: none;
  align-items: center;
  gap: 4px;
  height: var(--header-h);
  padding: 0 10px 0 14px;
  border-bottom: 1px solid var(--line);
}
.who {
  flex: none;
  margin-right: 6px;
}
.title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  font-size: 14px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.act {
  display: grid;
  flex: none;
  place-items: center;
  width: 32px;
  height: 32px;
  border-radius: 8px;
  color: var(--text-2);
}
.act:hover {
  background: var(--surface);
  color: var(--text);
}
.hint {
  flex: none;
  margin: 0;
  padding: 6px 14px;
  border-bottom: 1px solid var(--line);
  background: var(--surface);
  color: var(--text-3);
  font-size: 12px;
}
.link {
  color: var(--link);
  font-size: 12px;
}
.frames {
  position: relative;
  flex: 1;
  min-height: 0;
}
/* All frames at full size; only the current one shows (one loading ahead
   waits under it). */
iframe {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  border: 0;
  background: #fff;
  visibility: hidden;
}
iframe.shown {
  visibility: inherit;
  animation: frame-in 0.2s ease-out;
}
@keyframes frame-in {
  from {
    opacity: 0;
  }
}
/* Until the page has loaded: a strip running along the top. */
.loading {
  position: absolute;
  top: 0;
  right: 0;
  left: 0;
  height: 2px;
  overflow: hidden;
  pointer-events: none;
}
.loading::before {
  content: '';
  position: absolute;
  inset: 0;
  width: 40%;
  background: var(--accent);
  animation: loading 1.1s ease-in-out infinite;
}
@keyframes loading {
  from {
    transform: translateX(-100%);
  }
  to {
    transform: translateX(250%);
  }
}
@media (prefers-reduced-motion: reduce) {
  .reader,
  .reader.open {
    transition: visibility 0s;
  }
  iframe.shown {
    animation: none;
  }
  .loading::before {
    animation-duration: 3s;
  }
}
.shield {
  position: absolute;
  inset: 0;
  cursor: col-resize;
}
</style>
