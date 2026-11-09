<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api, channelTags, meta, tagsInfo } from '../api'
import ChipBar, { type Chip } from '../components/ChipBar.vue'
import Waterfall from '../components/Waterfall.vue'
import type { FeedMode } from '../types'

const route = useRoute()
const router = useRouter()

const newSeed = () => Math.floor(Math.random() * 2 ** 31)
const seed = ref(newSeed())

interface FeedState {
  tab: FeedMode
  /** Directory tag channel (开往: tech, life, ...); '' = all. */
  tag: string
  cover: boolean
  q: string
  /** A post category from its feed, reached by clicking #tag on a post. */
  category: string
}

// This view stays alive (<KeepAlive>) while other pages are open, so it only
// follows the URL while it is the current route.
const state = ref<FeedState>({ tab: 'discover', tag: '', cover: false, q: '', category: '' })
watch(
  () => [route.name, route.query],
  () => {
    if (route.name !== 'home') return
    const str = (v: unknown) => (typeof v === 'string' ? v.trim() : '')
    const next: FeedState = {
      tab: route.query.tab === 'latest' ? 'latest' : 'discover',
      tag: str(route.query.tag),
      cover: route.query.cover === '1',
      q: str(route.query.q),
      category: str(route.query.category),
    }
    // Ignore changes that only touch ?p= (the open post).
    if (JSON.stringify(next) !== JSON.stringify(state.value)) state.value = next
  },
  { immediate: true, deep: true },
)

const s = computed(() => state.value)
const filtering = computed(() => !!(s.value.q || s.value.category))
const resetKey = computed(() =>
  filtering.value
    ? `q:${s.value.q}|c:${s.value.category}`
    : `${s.value.tab}|${s.value.tag}|${s.value.cover}|${seed.value}`,
)

const load = (cursor: string | null) =>
  filtering.value
    ? api.posts({ mode: 'latest', cursor, q: s.value.q || undefined, category: s.value.category || undefined })
    : api.posts({
        mode: s.value.tab,
        cursor,
        seed: seed.value,
        tag: s.value.tag || undefined,
        cover: s.value.cover,
      })

const tabs: { id: FeedMode; label: string }[] = [
  { id: 'discover', label: '推荐' },
  { id: 'latest', label: '最新' },
]

const chips = computed<Chip[]>(() => [
  { value: '', label: '全部' },
  ...channelTags(tagsInfo.value)
    .filter((t) => t.posts > 0)
    .map((t) => ({ value: t.tag, label: t.label })),
])

function go(patch: Partial<Pick<FeedState, 'tab' | 'tag' | 'cover'>>) {
  const next = { ...s.value, ...patch }
  const query: Record<string, string> = {}
  if (next.tab === 'latest') query.tab = 'latest'
  if (next.tag) query.tag = next.tag
  if (next.cover) query.cover = '1'
  router.push({ path: '/', query })
  window.scrollTo({ top: 0 })
}

function selectTab(id: FeedMode) {
  if (id === s.value.tab) {
    // Tapping the active tab again reshuffles, like pulling to refresh.
    if (id === 'discover') seed.value = newSeed()
    window.scrollTo({ top: 0, behavior: 'smooth' })
    return
  }
  go({ tab: id })
}
</script>

<template>
  <div class="container">
    <div v-if="filtering" class="filter-head">
      <span v-if="s.category">#{{ s.category }}</span>
      <span v-else>“{{ s.q }}” 的搜索结果</span>
      <RouterLink to="/" class="clear">清除</RouterLink>
    </div>
    <div v-else class="feed-nav">
      <nav class="tabs" aria-label="排序">
        <button
          v-for="t in tabs"
          :key="t.id"
          class="tab"
          :class="{ active: s.tab === t.id }"
          :aria-current="s.tab === t.id ? 'page' : undefined"
          @click="selectTab(t.id)"
        >
          {{ t.label }}
        </button>
      </nav>
      <ChipBar :chips="chips" :active="s.tag" label="频道" class="channels" @select="(tag) => go({ tag })">
        <span class="divider" aria-hidden="true" />
        <button class="toggle" :class="{ on: s.cover }" :aria-pressed="s.cover" @click="go({ cover: !s.cover })">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="3" y="4" width="18" height="16" rx="3" />
            <circle cx="9" cy="10" r="2" />
            <path d="m21 16-5-5-9 9" />
          </svg>
          只看有图
        </button>
      </ChipBar>
    </div>

    <Waterfall :load="load" :reset-key="resetKey">
      <template #empty>
        <div class="empty">
          <p v-if="filtering || s.tag || s.cover">没有符合条件的文章</p>
          <template v-else>
            <p>还没有文章。</p>
            <p class="sub">
              后台正在慢慢抓取 RSS（已收录 {{ meta?.stats.blogs ?? 0 }} 个博客），稍后刷新看看。
            </p>
          </template>
        </div>
      </template>
    </Waterfall>
  </div>
</template>

<style scoped>
.feed-nav {
  position: sticky;
  top: var(--header-h);
  z-index: 10;
  display: flex;
  align-items: center;
  gap: 16px;
  margin: 0 -24px 12px;
  padding: 8px 24px 12px;
  background: color-mix(in srgb, var(--bg-page) 92%, transparent);
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
}
.tabs {
  display: flex;
  flex: none;
  gap: 4px;
}
.tab {
  height: 34px;
  padding: 0 16px;
  border-radius: 999px;
  color: var(--text-2);
  font-size: 15px;
}
.tab:hover {
  color: var(--text);
}
.tab.active {
  background: var(--surface);
  color: var(--text);
  font-weight: 600;
}
.channels {
  flex: 1;
  min-width: 0;
}
.divider {
  flex: none;
  width: 1px;
  height: 16px;
  background: var(--line);
}
.toggle {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 30px;
  padding: 0 12px;
  border-radius: 999px;
  border: 1px solid var(--line);
  color: var(--text-2);
  font-size: 13px;
  white-space: nowrap;
}
.toggle.on {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}
.filter-head {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 12px 4px 16px;
  font-size: 16px;
  font-weight: 600;
}
.clear {
  color: var(--link);
  font-size: 14px;
  font-weight: 400;
}
.empty {
  text-align: center;
  color: var(--text-2);
}
.empty p {
  margin: 4px 0;
}
.empty .sub {
  color: var(--text-3);
  font-size: 13px;
}
@media (max-width: 639px) {
  .feed-nav {
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
    margin: 0 -6px 6px;
    padding: 2px 8px 8px;
  }
  .tabs {
    justify-content: center;
  }
  .tab {
    height: 30px;
  }
  .tab.active {
    background: none;
    position: relative;
  }
  .tab.active::after {
    content: '';
    position: absolute;
    left: 50%;
    bottom: 0;
    width: 18px;
    height: 3px;
    border-radius: 2px;
    background: var(--accent);
    transform: translateX(-50%);
  }
  .toggle {
    height: 28px;
  }
}
</style>
