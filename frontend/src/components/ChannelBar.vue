<script setup lang="ts">
// The home page's channel bar. Pinned channels stay at the left; the rest
// scroll sideways (swipe, mouse wheel, drag, or the arrows on a desktop).
// 全部 / 有图: every post, or only those with a picture, in any channel. The ▾
// button opens the manager: reorder, pin, hide, add channels (categories,
// types, tags).
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { tagsInfo } from '../api'
import {
  addChannel,
  allChannels,
  channel,
  COLUMN_CHOICES,
  MAX_PINNED,
  moveChannel,
  prefs,
  removeChannel,
  resetChannels,
  shownChannels,
  togglePin,
  type ChannelId,
  type Media,
} from '../channels'
import { useDragScroll } from '../composables/dragScroll'

const props = defineProps<{ active: ChannelId }>()
const emit = defineEmits<{ select: [id: ChannelId] }>()

const info = computed(() => tagsInfo.value)
const shown = computed(() => shownChannels(info.value))
const shownIds = computed(() => new Set([...shown.value.pinned, ...shown.value.rest].map((c) => c.id)))
/** The open channel when it is not in the bar (a tag or category from a link). */
const extra = computed(() => (shownIds.value.has(props.active) ? null : channel(props.active, info.value)))

function setMedia(m: Media) {
  prefs.value.media = m
}

const open = ref(false)
const note = ref('')
const tagText = ref('')

const hidden = computed(() => allChannels(info.value).filter((c) => !shownIds.value.has(c.id)))
const hiddenGroups = computed(() =>
  [
    { label: '频道', items: hidden.value.filter((c) => c.type === 'mode') },
    { label: '分类', items: hidden.value.filter((c) => c.type === 'category') },
    { label: '类型', items: hidden.value.filter((c) => c.type === 'kind') },
  ].filter((g) => g.items.length),
)
const ordered = computed(() => [...shown.value.pinned, ...shown.value.rest])
const isPinned = (id: ChannelId) => shown.value.pinned.some((c) => c.id === id)

function pin(id: ChannelId) {
  note.value = togglePin(id, info.value) ? '' : `最多置顶 ${MAX_PINNED} 个频道`
}

function addTag() {
  const name = tagText.value.trim().replace(/^#/, '')
  if (!name) return
  addChannel(`tag:${name}`, info.value)
  tagText.value = ''
}

function go(id: ChannelId) {
  open.value = false
  emit('select', id)
}

// Keep the active chip in view after a switch.
const scroller = ref<HTMLElement>()
const { canLeft, canRight, scrollByPage } = useDragScroll(scroller)
watch(
  () => props.active,
  async () => {
    await nextTick()
    scroller.value?.querySelector<HTMLElement>('.chip.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  },
)

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') open.value = false
}
watch(open, (o) => (o ? document.addEventListener('keydown', onKey) : document.removeEventListener('keydown', onKey)))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))
</script>

<template>
  <div class="channel-bar">
    <nav class="bar" aria-label="频道">
      <div v-if="shown.pinned.length" class="pinned">
        <button
          v-for="c in shown.pinned"
          :key="c.id"
          class="chip"
          :class="{ active: active === c.id }"
          :aria-current="active === c.id ? 'page' : undefined"
          @click="emit('select', c.id)"
        >
          {{ c.label }}
        </button>
      </div>
      <div class="scroll-wrap">
        <button v-if="canLeft" class="arrow left" tabindex="-1" aria-hidden="true" @click="scrollByPage(-1)">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round">
            <path d="m15 5-7 7 7 7" />
          </svg>
        </button>
        <div ref="scroller" class="scroller" :class="{ 'fade-left': canLeft, 'fade-right': canRight }">
          <button v-if="extra" class="chip active extra" aria-current="page" @click="emit('select', extra.id)">
            {{ extra.label }}
          </button>
          <button
            v-for="c in shown.rest"
            :key="c.id"
            class="chip"
            :class="{ active: active === c.id }"
            :aria-current="active === c.id ? 'page' : undefined"
            @click="emit('select', c.id)"
          >
            {{ c.label }}
          </button>
        </div>
        <button v-if="canRight" class="arrow right" tabindex="-1" aria-hidden="true" @click="scrollByPage(1)">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round">
            <path d="m9 5 7 7-7 7" />
          </svg>
        </button>
      </div>
      <div class="media" role="group" aria-label="筛选文章">
        <button
          class="media-btn"
          :class="{ on: prefs.media === 'all' }"
          :aria-pressed="prefs.media === 'all'"
          title="全部文章"
          @click="setMedia('all')"
        >
          全部
        </button>
        <button
          class="media-btn"
          :class="{ on: prefs.media === 'cover' }"
          :aria-pressed="prefs.media === 'cover'"
          title="只看有图的文章"
          @click="setMedia('cover')"
        >
          有图
        </button>
        <button
          class="media-btn"
          :class="{ on: prefs.media === 'text' }"
          :aria-pressed="prefs.media === 'text'"
          title="不看摘要很短（不到 20 个字）的文章"
          @click="setMedia('text')"
        >
          有摘要
        </button>
      </div>
      <button class="more" :class="{ open }" :aria-expanded="open" aria-label="管理频道" @click="open = !open">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="m6 9 6 6 6-6" />
        </svg>
      </button>
    </nav>

    <div v-if="open" class="backdrop" @click="open = false" />
    <section v-if="open" class="manager" aria-label="频道管理">
      <header class="m-head">
        <h2>频道管理</h2>
        <button class="link" @click="resetChannels()">恢复默认</button>
        <button class="btn btn-ghost done" @click="open = false">完成</button>
      </header>

      <p class="hint">
        置顶的频道固定在左侧，最多 {{ MAX_PINNED }} 个；第一个频道是打开首页时显示的频道。
        <span v-if="note" class="warn">{{ note }}</span>
      </p>
      <ol class="mine">
        <li v-for="(c, i) in ordered" :key="c.id" :class="{ pinned: isPinned(c.id) }">
          <button class="name" @click="go(c.id)">{{ c.title }}</button>
          <button
            class="icon"
            :class="{ on: isPinned(c.id) }"
            :aria-pressed="isPinned(c.id)"
            :title="isPinned(c.id) ? '取消置顶' : '置顶'"
            @click="pin(c.id)"
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round">
              <path d="M9 4h6l-1 5 4 4H6l4-4-1-5zM12 13v7" />
            </svg>
          </button>
          <button class="icon" title="上移" :disabled="i === 0" @click="moveChannel(c.id, -1, info)">↑</button>
          <button class="icon" title="下移" :disabled="i === ordered.length - 1" @click="moveChannel(c.id, 1, info)">↓</button>
          <button class="icon" title="移出频道栏" :disabled="ordered.length === 1" @click="removeChannel(c.id, info)">✕</button>
        </li>
      </ol>

      <template v-for="g in hiddenGroups" :key="g.label">
        <h3>添加{{ g.label }}</h3>
        <p class="adds">
          <button v-for="c in g.items" :key="c.id" class="add" @click="addChannel(c.id, info)">+ {{ c.title }}</button>
        </p>
      </template>

      <h3>添加标签频道</h3>
      <form class="tag-form" @submit.prevent="addTag">
        <input v-model="tagText" placeholder="输入标签，例如 Rust" aria-label="标签" />
        <button class="btn btn-ghost" type="submit">添加</button>
      </form>

      <h3>每行几列</h3>
      <p class="adds" role="group" aria-label="每行几列">
        <button
          v-for="n in COLUMN_CHOICES"
          :key="String(n)"
          class="add col-choice"
          :class="{ on: prefs.columns === n }"
          :aria-pressed="prefs.columns === n"
          @click="prefs.columns = n"
        >
          {{ n ?? '自动' }}
        </button>
      </p>
      <p class="hint">首页、博客页、搜索和「我的」里的瀑布流都按这个列数排；屏幕不够宽时会少几列，免得卡片太窄。</p>
    </section>
  </div>
</template>

<style scoped>
.channel-bar {
  position: relative;
}
.bar {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.pinned {
  display: flex;
  flex: none;
  gap: 8px;
  padding-right: 8px;
  border-right: 1px solid var(--line);
}
.scroll-wrap {
  position: relative;
  display: flex;
  flex: 1;
  min-width: 0;
}
.scroller {
  display: flex;
  flex: 1;
  gap: 8px;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
  -webkit-overflow-scrolling: touch;
  user-select: none;
}
/* Fade the edges that have more behind them. */
.scroller.fade-left {
  mask-image: linear-gradient(to right, transparent, #000 40px);
}
.scroller.fade-right {
  mask-image: linear-gradient(to right, #000 calc(100% - 40px), transparent);
}
.scroller.fade-left.fade-right {
  mask-image: linear-gradient(to right, transparent, #000 40px, #000 calc(100% - 40px), transparent);
}
.scroller.dragging,
.scroller.dragging * {
  cursor: grabbing;
}
.arrow {
  position: absolute;
  top: 50%;
  z-index: 1;
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  border: 1px solid var(--line);
  background: var(--bg);
  color: var(--text-2);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.12);
  transform: translateY(-50%);
}
.arrow:hover {
  color: var(--text);
}
.arrow.left {
  left: 0;
}
.arrow.right {
  right: 0;
}
/* Touch screens swipe instead. */
@media (hover: none) {
  .arrow {
    display: none;
  }
}
.scroller::-webkit-scrollbar {
  display: none;
}
.chip {
  flex: none;
  height: 32px;
  padding: 0 14px;
  border-radius: 999px;
  color: var(--text-2);
  font-size: 15px;
  white-space: nowrap;
  transition:
    background 0.15s,
    color 0.15s;
}
.chip:hover {
  color: var(--text);
}
.chip.active {
  background: var(--surface);
  color: var(--text);
  font-weight: 600;
}
.chip.extra {
  outline: 1px dashed var(--line);
}
.media {
  display: flex;
  flex: none;
  padding: 2px;
  border-radius: 999px;
  background: var(--surface);
}
.media-btn {
  height: 26px;
  padding: 0 10px;
  border-radius: 999px;
  color: var(--text-2);
  font-size: 13px;
  white-space: nowrap;
}
.media-btn:hover {
  color: var(--text);
}
.media-btn.on {
  background: var(--bg);
  color: var(--text);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
}
.more {
  display: grid;
  flex: none;
  place-items: center;
  width: 32px;
  height: 32px;
  border-radius: 50%;
  color: var(--text-2);
  transition: transform 0.15s;
}
.more:hover {
  background: var(--surface);
}
.more.open {
  transform: rotate(180deg);
}
.backdrop {
  position: fixed;
  inset: 0;
  z-index: 29;
}
.manager {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  z-index: 30;
  width: min(560px, 100%);
  max-height: min(70vh, 640px);
  overflow-y: auto;
  padding: 14px 16px 18px;
  border-radius: 16px;
  border: 1px solid var(--line);
  background: var(--bg);
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.18);
  overscroll-behavior: contain;
}
.m-head {
  display: flex;
  align-items: center;
  gap: 12px;
}
.m-head h2 {
  flex: 1;
  margin: 0;
  font-size: 16px;
}
.link {
  color: var(--link);
  font-size: 13px;
}
.done {
  height: 30px;
}
.hint {
  margin: 6px 0 10px;
  color: var(--text-3);
  font-size: 12px;
}
.warn {
  color: var(--accent);
}
.mine {
  margin: 0;
  padding: 0;
  list-style: none;
}
.mine li {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 2px 0;
  border-bottom: 1px solid var(--line);
}
.mine .name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  padding: 8px 4px;
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 14px;
}
.mine li.pinned .name {
  font-weight: 600;
}
.icon {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  border-radius: 8px;
  color: var(--text-3);
  font-size: 14px;
}
.icon:hover:not(:disabled) {
  background: var(--surface);
  color: var(--text);
}
.icon:disabled {
  opacity: 0.3;
  cursor: default;
}
.icon.on {
  color: var(--accent);
}
.manager h3 {
  margin: 16px 0 8px;
  color: var(--text-2);
  font-size: 13px;
  font-weight: 600;
}
.adds {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 0;
}
.add {
  padding: 3px 10px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
}
.add:hover {
  color: var(--text);
}
.col-choice {
  min-width: 34px;
}
.col-choice.on {
  background: var(--text);
  color: var(--bg);
}
.tag-form {
  display: flex;
  gap: 8px;
}
.tag-form input {
  flex: 1;
  min-width: 0;
  height: 34px;
  padding: 0 14px;
  border: 0;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-size: 14px;
  outline: none;
}
@media (max-width: 639px) {
  .chip {
    height: 30px;
    padding: 0 10px;
    font-size: 14px;
  }
  .chip.active {
    background: none;
    position: relative;
  }
  .chip.active::after {
    content: '';
    position: absolute;
    left: 50%;
    bottom: 0;
    width: 16px;
    height: 3px;
    border-radius: 2px;
    background: var(--accent);
    transform: translateX(-50%);
  }
  .pinned {
    gap: 2px;
    padding-right: 4px;
  }
  .scroller {
    gap: 2px;
  }
  .manager {
    position: fixed;
    top: auto;
    bottom: 0;
    left: 0;
    right: 0;
    width: 100%;
    max-height: 80vh;
    padding-bottom: calc(18px + var(--safe-b));
    border-radius: 16px 16px 0 0;
  }
  .backdrop {
    background: var(--overlay);
  }
}
</style>
