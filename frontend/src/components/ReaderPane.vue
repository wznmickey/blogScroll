<script setup lang="ts">
// The right part of a split page: a blog's own page in a frame, with a bar to
// like or save the post, reload, open it in a new tab, or close. The divider
// can be dragged.
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { closeReader, markBlocked, readerOpen, readerPage, readerShare, setReaderShare } from '../reader'
import BlogAvatar from './BlogAvatar.vue'
import LibraryButton from './LibraryButton.vue'

const route = useRoute()
const frameKey = ref(0)
const resizing = ref(false)

// A new page starts from the top, not where the last one was.
watch(
  () => readerPage.value?.url,
  () => frameKey.value++,
)

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
  <aside v-if="readerOpen && readerPage" class="reader" aria-label="原文">
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
    <header class="bar">
      <RouterLink :to="`/blog/${readerPage.blog.id}`" class="who" :title="readerPage.blog.name">
        <BlogAvatar :name="readerPage.blog.name" :icon="readerPage.blog.icon" :size="24" />
      </RouterLink>
      <span class="title" :title="readerPage.title">{{ readerPage.title }}</span>
      <template v-if="readerPage.post">
        <LibraryButton :post="readerPage.post" list="likes" :size="18" class="act" />
        <LibraryButton :post="readerPage.post" list="saves" :size="18" class="act" />
      </template>
      <button class="act" title="重新加载" aria-label="重新加载" @click="frameKey++">
        <svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7" />
        </svg>
      </button>
      <a :href="readerPage.url" target="_blank" rel="noopener" class="act" title="在新标签页打开" aria-label="在新标签页打开">
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
    <p v-if="readerPage.blog.framable == null" class="hint">
      页面空白或显示「拒绝连接」？有的网站不允许被嵌入。
      <button class="link" @click="giveUp">改用新标签页打开</button>
    </p>
    <iframe
      :key="frameKey"
      :src="readerPage.url"
      :title="readerPage.title"
      sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox allow-forms allow-modals"
      allow="fullscreen"
    />
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
  z-index: 45;
  display: flex;
  flex-direction: column;
  width: var(--pane-w);
  border-left: 1px solid var(--line);
  background: var(--bg);
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
iframe {
  flex: 1;
  width: 100%;
  border: 0;
  background: #fff;
}
.shield {
  position: absolute;
  inset: 0;
  cursor: col-resize;
}
</style>
