<script setup lang="ts">
import { computed, onActivated, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api, meta, tagLabel, tagsInfo } from '../api'
import BlogAvatar from '../components/BlogAvatar.vue'
import ChipBar, { type Chip } from '../components/ChipBar.vue'
import FollowButton from '../components/FollowButton.vue'
import { follows } from '../follow'
import { blogPath, hostOf, relativeTime } from '../format'
import type { BlogSort, BlogSummary } from '../types'

const route = useRoute()
const router = useRouter()

const FOLLOWING = '@following'

// Kept alive while a blog page is open (so going back keeps the list and
// scroll position); only follow the URL while this is the current route.
const dirtag = ref('')
/** A top-level category id, or FOLLOWING. */
const category = ref('')
const kind = ref('')
const sort = ref<BlogSort>('recent')
const q = ref('')
watch(
  () => [route.name, route.query],
  () => {
    if (route.name !== 'blogs') return
    const str = (v: unknown) => (typeof v === 'string' ? v : '')
    dirtag.value = str(route.query.dirtag)
    category.value = route.query.following === '1' ? FOLLOWING : str(route.query.category)
    kind.value = str(route.query.kind)
    sort.value = route.query.sort === 'posts' || route.query.sort === 'name' ? route.query.sort : 'recent'
    q.value = str(route.query.q)
  },
  { immediate: true, deep: true },
)
const followingOnly = computed(() => category.value === FOLLOWING)
// The list of followed blogs is read when the filter is applied, so
// unfollowing one in the list does not make it vanish under the cursor.
const followIds = ref<number[]>([])
const text = ref(q.value)

const blogs = ref<BlogSummary[]>([])
const offset = ref<number | null>(0)
const loading = ref(false)
const error = ref(false)
const sentinel = ref<HTMLElement>()
let generation = 0

const chips = computed<Chip[]>(() => [
  { value: '', label: '全部', count: meta.value?.stats.blogs },
  { value: FOLLOWING, label: '我的关注', count: follows.value.length },
  ...(tagsInfo.value?.categories ?? [])
    .filter((c) => (c.blogs ?? 0) > 0)
    .map((c) => ({ value: c.id, label: c.label, count: c.blogs })),
])
const kinds = computed(() => tagsInfo.value?.kinds ?? [])
const sorts: { id: BlogSort; label: string }[] = [
  { id: 'recent', label: '最近更新' },
  { id: 'posts', label: '文章最多' },
  { id: 'name', label: '名称' },
]

function setQuery(patch: Record<string, string>) {
  const query: Record<string, string> = {
    dirtag: dirtag.value,
    category: followingOnly.value ? '' : category.value,
    following: followingOnly.value ? '1' : '',
    kind: kind.value,
    sort: sort.value,
    q: q.value,
    ...patch,
  }
  if (query.category === FOLLOWING) {
    query.category = ''
    query.following = '1'
  } else if ('category' in patch) {
    query.following = ''
  }
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
    if (followingOnly.value && !followIds.value.length) {
      offset.value = null
      return
    }
    const page = await api.blogs({
      offset: offset.value,
      dirtag: dirtag.value || undefined,
      category: followingOnly.value ? undefined : category.value || undefined,
      kind: kind.value || undefined,
      ids: followingOnly.value ? followIds.value : undefined,
      sort: sort.value,
      q: q.value || undefined,
    })
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
  () => [dirtag.value, category.value, kind.value, sort.value, q.value],
  () => {
    followIds.value = [...follows.value]
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
        <input v-model="text" type="search" placeholder="搜索博主名、网址或简介" aria-label="搜索博主" />
        <select
          v-if="kinds.length > 1"
          :value="kind"
          aria-label="类型"
          @change="setQuery({ kind: ($event.target as HTMLSelectElement).value })"
        >
          <option value="">全部类型</option>
          <option v-for="k in kinds" :key="k.id" :value="k.id">{{ k.label }} {{ k.blogs }}</option>
        </select>
        <select :value="sort" aria-label="排序" @change="setQuery({ sort: ($event.target as HTMLSelectElement).value })">
          <option v-for="o in sorts" :key="o.id" :value="o.id">{{ o.label }}</option>
        </select>
      </div>
    </header>
    <ChipBar :chips="chips" :active="category" label="按分类筛选" class="filters" @select="(c) => setQuery({ category: c })" />
    <p v-if="dirtag" class="tag-note">
      开往标签：{{ tagLabel(dirtag) }}
      <button type="button" @click="setQuery({ dirtag: '' })">清除</button>
    </p>

    <ul class="grid">
      <li v-for="b in blogs" :key="b.id">
        <RouterLink :to="blogPath(b)" class="card">
          <BlogAvatar :name="b.name" :icon="b.icon" :size="48" />
          <div class="info">
            <div class="name-row">
              <h2>{{ b.name }}</h2>
              <FollowButton :blog-id="b.id" :blog-key="b.key" small />
            </div>
            <p class="host">{{ hostOf(b.homepage) }}</p>
            <p v-if="b.description" class="desc">{{ b.description }}</p>
            <p class="tags">
              <span v-if="b.kind && b.kind.id !== 'blog'" class="kind">{{ b.kind.label }}</span>
              <span v-for="c in b.categories ?? []" :key="c.id" class="topic">{{ c.label }}</span>
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
      <p v-else-if="!blogs.length && followingOnly" class="hint">还没有关注任何博主，点卡片上的「关注」试试</p>
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
  grid-template-columns: repeat(auto-fill, minmax(min(280px, 100%), 1fr));
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
  flex: 1;
  min-width: 0;
}
.name-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.tags .topic {
  background: color-mix(in srgb, var(--link) 12%, transparent);
  color: var(--link);
}
.tags .kind {
  background: var(--accent-soft);
  color: var(--accent);
}
.tag-note {
  margin: -8px 4px 14px;
  color: var(--text-2);
  font-size: 13px;
}
.tag-note button {
  margin-left: 8px;
  color: var(--link);
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
    flex-wrap: wrap;
    width: 100%;
  }
  /* The search box gets its own line; the two selects share the next. */
  .controls input {
    flex: 1 1 100%;
    width: auto;
    min-width: 0;
  }
  .controls select {
    flex: 1 1 0;
    min-width: 0;
  }
  .filters {
    padding: 0 6px 12px;
  }
  .grid {
    grid-template-columns: minmax(0, 1fr);
    gap: 6px;
  }
  .card {
    padding: 12px;
    border-radius: 10px;
  }
}
</style>
