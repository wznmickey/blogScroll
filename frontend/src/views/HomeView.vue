<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api, meta } from '../api'
import Waterfall from '../components/Waterfall.vue'
import type { FeedMode } from '../types'

const route = useRoute()
const router = useRouter()

const newSeed = () => Math.floor(Math.random() * 2 ** 31)
const seed = ref(newSeed())

// This view stays alive (<KeepAlive>) while other pages are open, so it only
// follows the URL while it is the current route.
const tab = ref<FeedMode>('discover')
const q = ref('')
watch(
  () => [route.name, route.query.tab, route.query.q],
  () => {
    if (route.name !== 'home') return
    tab.value = route.query.tab === 'latest' ? 'latest' : 'discover'
    q.value = typeof route.query.q === 'string' ? route.query.q.trim() : ''
  },
  { immediate: true },
)
// Only these drive a reload; ?p= (the open post) must not.
const resetKey = computed(() => (q.value ? `q:${q.value}` : `${tab.value}:${seed.value}`))

const load = (cursor: string | null) =>
  api.posts({ mode: q.value ? 'latest' : tab.value, cursor, seed: seed.value, q: q.value || undefined })

const tabs: { id: FeedMode; label: string }[] = [
  { id: 'discover', label: '推荐' },
  { id: 'latest', label: '最新' },
]

function select(id: FeedMode) {
  if (id === tab.value) {
    // Tapping the active tab again reshuffles, like pulling to refresh.
    if (id === 'discover') seed.value = newSeed()
    window.scrollTo({ top: 0, behavior: 'smooth' })
    return
  }
  router.push({ query: id === 'latest' ? { tab: 'latest' } : {} })
  window.scrollTo({ top: 0 })
}
</script>

<template>
  <div class="container">
    <div v-if="q" class="search-head">
      <span>“{{ q }}” 的搜索结果</span>
      <RouterLink to="/" class="clear">清除</RouterLink>
    </div>
    <nav v-else class="channels" aria-label="频道">
      <button
        v-for="t in tabs"
        :key="t.id"
        class="channel"
        :class="{ active: tab === t.id }"
        :aria-current="tab === t.id ? 'page' : undefined"
        @click="select(t.id)"
      >
        {{ t.label }}
      </button>
    </nav>

    <Waterfall :load="load" :reset-key="resetKey">
      <template #empty>
        <div class="empty">
          <p v-if="q">没有找到相关文章</p>
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
.channels {
  position: sticky;
  top: var(--header-h);
  z-index: 10;
  display: flex;
  gap: 8px;
  margin: 0 -24px 12px;
  padding: 8px 24px 12px;
  background: color-mix(in srgb, var(--bg-page) 92%, transparent);
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
}
.channel {
  height: 34px;
  padding: 0 16px;
  border-radius: 999px;
  color: var(--text-2);
  font-size: 15px;
}
.channel:hover {
  color: var(--text);
}
.channel.active {
  background: var(--surface);
  color: var(--text);
  font-weight: 600;
}
.search-head {
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
  .channels {
    justify-content: center;
    margin: 0 -6px 6px;
    padding: 4px 6px 8px;
  }
  .channel {
    height: 30px;
    font-size: 15px;
  }
  .channel.active {
    background: none;
    position: relative;
  }
  .channel.active::after {
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
}
</style>
