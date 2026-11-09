<script setup lang="ts">
import { computed, onActivated, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api, channelTags, meta, tagsInfo } from '../api'
import BlogAvatar from '../components/BlogAvatar.vue'
import ChipBar, { type Chip } from '../components/ChipBar.vue'
import { hostOf, relativeTime } from '../format'
import type { BlogSort, BlogSummary } from '../types'

const route = useRoute()
const router = useRouter()

// Kept alive while a blog page is open (so going back keeps the list and
// scroll position); only follow the URL while this is the current route.
const tag = ref('')
const sort = ref<BlogSort>('recent')
const q = ref('')
watch(
  () => [route.name, route.query],
  () => {
    if (route.name !== 'blogs') return
    const str = (v: unknown) => (typeof v === 'string' ? v : '')
    tag.value = str(route.query.tag)
    sort.value = route.query.sort === 'posts' || route.query.sort === 'name' ? route.query.sort : 'recent'
    q.value = str(route.query.q)
  },
  { immediate: true, deep: true },
)
const text = ref(q.value)

const blogs = ref<BlogSummary[]>([])
const offset = ref<number | null>(0)
const loading = ref(false)
const error = ref(false)
const sentinel = ref<HTMLElement>()
let generation = 0

const chips = computed<Chip[]>(() => [
  { value: '', label: '全部', count: meta.value?.stats.blogs },
  ...channelTags(tagsInfo.value).map((t) => ({ value: t.tag, label: t.label, count: t.blogs })),
])
const sorts: { id: BlogSort; label: string }[] = [
  { id: 'recent', label: '最近更新' },
  { id: 'posts', label: '文章最多' },
  { id: 'name', label: '名称' },
]

function setQuery(patch: Record<string, string>) {
  const query: Record<string, string> = { tag: tag.value, sort: sort.value, q: q.value, ...patch }
  if (query.sort === 'recent') delete query.sort
  for (const k of Object.keys(query)) if (!query[k]) delete query[k]
  router.replace({ query })
}

let debounce: ReturnType<typeof setTimeout> | undefined
watch(text, (v) => {
  clearTimeout(debounce)
  debounce = setTimeout(() => setQuery({ q: v.trim() }), 300)
})

async function loadMore() {
  if (loading.value || offset.value === null) return
  const gen = generation
  loading.value = true
  error.value = false
  try {
    const page = await api.blogs({ offset: offset.value, tag: tag.value || undefined, sort: sort.value, q: q.value || undefined })
    if (gen !== generation) return
    blogs.value.push(...page.items)
    offset.value = page.next_cursor ? Number(page.next_cursor) : null
  } catch {
    if (gen === generation) error.value = true
  } finally {
    if (gen === generation) loading.value = false
  }
}

watch(
  () => [tag.value, sort.value, q.value],
  () => {
    generation++
    blogs.value = []
    offset.value = 0
    loading.value = false
    void loadMore()
  },
  { immediate: true },
)

let observer: IntersectionObserver | undefined
onActivated(() => (document.title = '博客'))
onMounted(() => {
  observer = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && void loadMore(), {
    rootMargin: '0px 0px 800px 0px',
  })
  observer.observe(sentinel.value!)
})
onBeforeUnmount(() => {
  observer?.disconnect()
  clearTimeout(debounce)
})
</script>

<template>
  <div class="container">
    <header class="head">
      <h1>博客</h1>
      <div class="controls">
        <input v-model="text" type="search" placeholder="搜索博客名或网址" aria-label="搜索博客" />
        <select :value="sort" aria-label="排序" @change="setQuery({ sort: ($event.target as HTMLSelectElement).value })">
          <option v-for="o in sorts" :key="o.id" :value="o.id">{{ o.label }}</option>
        </select>
      </div>
    </header>
    <ChipBar :chips="chips" :active="tag" label="按标签筛选" class="filters" @select="(t) => setQuery({ tag: t })" />

    <ul class="grid">
      <li v-for="b in blogs" :key="b.id">
        <RouterLink :to="`/blog/${b.id}`" class="card">
          <BlogAvatar :name="b.name" :icon="b.icon" :size="48" />
          <div class="info">
            <h2>{{ b.name }}</h2>
            <p class="host">{{ hostOf(b.homepage) }}</p>
            <p v-if="b.description" class="desc">{{ b.description }}</p>
            <p class="tags">
              <span v-for="t in b.tags" :key="t.directory + t.tag">{{ t.label }}</span>
            </p>
            <p class="stats">
              <template v-if="b.post_count">
                {{ b.post_count }} 篇<template v-if="b.last_post_at"> · {{ relativeTime(b.last_post_at) }}更新</template>
              </template>
              <template v-else-if="b.has_feed">暂无近期文章</template>
              <template v-else>暂未找到 RSS</template>
            </p>
          </div>
        </RouterLink>
      </li>
    </ul>

    <div ref="sentinel" class="status">
      <span v-if="loading" class="spinner" aria-label="加载中" />
      <button v-else-if="error" class="btn btn-ghost" @click="loadMore">加载失败，点此重试</button>
      <p v-else-if="!blogs.length" class="hint">没有找到博客</p>
      <p v-else-if="offset === null" class="hint">— 共 {{ blogs.length }} 个 —</p>
    </div>
  </div>
</template>

<style scoped>
.head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 16px 4px 12px;
}
h1 {
  margin: 0;
  font-size: 22px;
}
.controls {
  display: flex;
  gap: 8px;
}
.controls input,
.controls select {
  height: 34px;
  padding: 0 14px;
  border: 0;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  outline: none;
}
.controls input {
  width: 220px;
}
.filters {
  padding: 0 4px 16px;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 12px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.card {
  display: flex;
  gap: 14px;
  height: 100%;
  padding: 16px;
  border-radius: 14px;
  background: var(--card);
  border: 1px solid var(--line);
  transition: border-color 0.15s;
}
.card:hover {
  border-color: color-mix(in srgb, var(--text) 25%, transparent);
}
.info {
  min-width: 0;
}
h2 {
  margin: 0;
  font-size: 15px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.host {
  margin: 0;
  color: var(--text-3);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.desc {
  margin: 6px 0 0;
  color: var(--text-2);
  font-size: 12px;
  line-height: 1.5;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin: 8px 0 0;
}
.tags span {
  padding: 0 8px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 11px;
  line-height: 20px;
}
.stats {
  margin: 6px 0 0;
  color: var(--text-2);
  font-size: 12px;
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
  .head {
    padding: 12px 6px 8px;
  }
  .controls {
    width: 100%;
  }
  .controls input {
    flex: 1;
    width: auto;
  }
  .filters {
    padding: 0 6px 12px;
  }
  .grid {
    grid-template-columns: 1fr;
    gap: 6px;
  }
  .card {
    padding: 12px;
    border-radius: 10px;
  }
}
</style>
