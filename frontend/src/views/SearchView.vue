<script setup lang="ts">
// One search over everything: posts (server), bloggers, categories and tags,
// and this reader's own saves, likes and history (in the browser).
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { allCategories, api, categoryName, tagsInfo } from '../api'
import BlogAvatar from '../components/BlogAvatar.vue'
import ChipBar, { type Chip } from '../components/ChipBar.vue'
import FollowButton from '../components/FollowButton.vue'
import PostRow from '../components/PostRow.vue'
import Waterfall from '../components/Waterfall.vue'
import { blogPath, hostOf } from '../format'
import { readerFilters } from '../options'
import { track } from '../telemetry'
import { LIST_LABELS, recordShown, searchList, type ListName } from '../library'
import type { BlogSummary, CategoryNode, Page, PostCard, TagStat } from '../types'

type SearchType = 'all' | 'posts' | 'blogs' | 'tags' | ListName

const route = useRoute()
const router = useRouter()
const TYPES: SearchType[] = ['all', 'posts', 'blogs', 'tags', 'saves', 'likes', 'history', 'seen']
const q = computed(() => (typeof route.query.q === 'string' ? route.query.q.trim() : ''))
const type = computed<SearchType>(() => {
  const t = route.query.type as SearchType
  return TYPES.includes(t) ? t : 'all'
})

// Local lists are searched as they are now; the chips show their counts.
const mine = computed(() => ({
  saves: searchList('saves', q.value),
  likes: searchList('likes', q.value),
  history: searchList('history', q.value),
  seen: searchList('seen', q.value),
}))

const categories = computed<CategoryNode[]>(() => {
  const n = q.value.toLowerCase()
  if (!n) return []
  return allCategories(tagsInfo.value).filter(
    (c) => c.id !== 'other' && (c.label.toLowerCase().includes(n) || n.includes(c.label.toLowerCase())),
  )
})

const tags = ref<TagStat[]>([])
const blogs = ref<BlogSummary[]>([])
const moreBlogs = ref(false)
const blogOffset = ref(0)
const loadingBlogs = ref(false)

async function loadBlogs(reset: boolean) {
  const needle = q.value
  if (reset) {
    blogs.value = []
    blogOffset.value = 0
    moreBlogs.value = false
  }
  if (!needle) return
  loadingBlogs.value = true
  try {
    const limit = type.value === 'blogs' ? 24 : 6
    const page = await api.blogs({ q: needle, limit, offset: blogOffset.value })
    if (needle !== q.value) return
    blogs.value.push(...page.items)
    blogOffset.value += page.items.length
    moreBlogs.value = !!page.next_cursor
  } catch {
    // The other sections still show.
  } finally {
    loadingBlogs.value = false
  }
}

let counted = ''
watch(
  () => [q.value, type.value],
  async () => {
    if (route.name !== 'search') return
    document.title = q.value ? `搜索：${q.value}` : '搜索'
    tags.value = []
    // What readers look for (大家在看), once per search.
    if (q.value && q.value !== counted) track('search', (counted = q.value))
    void loadBlogs(true)
    if (!q.value) return
    try {
      const needle = q.value
      const found = await api.searchTags(needle, type.value === 'tags' ? 200 : 12)
      if (needle === q.value) tags.value = found
    } catch {
      // As above.
    }
  },
  { immediate: true },
)

const chips = computed<Chip[]>(() => [
  { value: 'all', label: '全部' },
  { value: 'posts', label: '文章' },
  { value: 'blogs', label: '博主' },
  { value: 'tags', label: '分类与标签' },
  ...(['saves', 'likes', 'history', 'seen'] as const).map((n) => ({
    value: n,
    label: LIST_LABELS[n],
    count: mine.value[n].length,
  })),
])

function setType(t: string) {
  router.replace({ query: { ...route.query, type: t === 'all' ? undefined : t } })
  window.scrollTo({ top: 0 })
}

const loadPosts = (cursor: string | null) => api.posts({ mode: 'latest', q: q.value, cursor, ...readerFilters() })

function localPages(name: ListName) {
  return async (cursor: string | null): Promise<Page<PostCard>> => {
    const all = mine.value[name].map((e) => e.post)
    const offset = Number(cursor) || 0
    return { items: all.slice(offset, offset + 30), next_cursor: offset + 30 < all.length ? String(offset + 30) : null }
  }
}
const localLoad = computed(() => (type.value in LIST_LABELS ? localPages(type.value as ListName) : null))
</script>

<template>
  <div class="container">
    <header class="head">
      <h1 v-if="q">“{{ q }}”</h1>
      <h1 v-else>搜索</h1>
      <ChipBar :chips="chips" :active="type" label="搜索范围" class="types" @select="setType" />
    </header>

    <p v-if="!q" class="hint">在上方输入关键词，可以搜索文章、博主、分类和标签，以及你的收藏、喜欢和浏览历史。</p>

    <template v-else>
      <!-- 分类与标签 -->
      <section v-if="(type === 'all' || type === 'tags') && (categories.length || tags.length)" class="section">
        <h2>分类与标签</h2>
        <p class="chips">
          <RouterLink
            v-for="c in categories"
            :key="c.id"
            :to="{ path: '/', query: { c: `cat:${c.id}` } }"
            class="chip cat"
            >{{ categoryName(c.id) }}<span class="n">{{ c.posts }}</span></RouterLink
          >
          <RouterLink v-for="t in tags" :key="t.name" :to="{ path: '/', query: { c: `tag:${t.name}` } }" class="chip"
            >#{{ t.name }}<span class="n">{{ t.posts }}</span></RouterLink
          >
        </p>
      </section>
      <p v-else-if="type === 'tags'" class="hint">没有匹配的分类或标签</p>

      <!-- 博主 -->
      <section v-if="(type === 'all' || type === 'blogs') && blogs.length" class="section">
        <h2>
          博主
          <button v-if="type === 'all' && moreBlogs" class="all" @click="setType('blogs')">查看全部</button>
        </h2>
        <ul class="blog-hits">
          <li v-for="b in blogs" :key="b.id">
            <RouterLink :to="blogPath(b)" class="hit">
              <BlogAvatar :name="b.name" :icon="b.icon" :size="40" />
              <span class="hit-text">
                <span class="hit-name">{{ b.name }}</span>
                <span class="hit-sub">{{ b.description || hostOf(b.homepage) }}</span>
              </span>
              <FollowButton :blog-id="b.id" :blog-key="b.key" small />
            </RouterLink>
          </li>
        </ul>
        <button v-if="type === 'blogs' && moreBlogs" class="btn btn-ghost more" :disabled="loadingBlogs" @click="loadBlogs(false)">
          加载更多
        </button>
      </section>
      <p v-else-if="type === 'blogs' && !loadingBlogs" class="hint">没有找到博主</p>

      <!-- 收藏 / 喜欢 / 历史 -->
      <template v-if="type === 'all'">
        <template v-for="n in (['saves', 'likes', 'history', 'seen'] as const)" :key="n">
          <section v-if="mine[n].length" class="section">
            <h2>
              {{ n === 'seen' ? '刷到过' : `我的${LIST_LABELS[n]}` }}
              <button v-if="mine[n].length > 4" class="all" @click="setType(n)">查看全部 {{ mine[n].length }} 篇</button>
            </h2>
            <ul class="rows">
              <li v-for="e in mine[n].slice(0, 4)" :key="e.post.id"><PostRow :post="e.post" :size="48" /></li>
            </ul>
          </section>
        </template>
      </template>

      <!-- 文章 -->
      <section v-if="type === 'all' || type === 'posts'" class="section">
        <h2 v-if="type === 'all'">文章</h2>
        <Waterfall :load="loadPosts" :reset-key="`posts|${q}`" @impression="(p) => recordShown(p, `搜索：${q}`)">
          <template #empty><p class="hint">没有找到文章</p></template>
        </Waterfall>
      </section>
      <Waterfall v-else-if="localLoad" :load="localLoad" :reset-key="`${type}|${q}`">
        <template #empty><p class="hint">没有匹配的文章</p></template>
      </Waterfall>
    </template>
  </div>
</template>

<style scoped>
.head {
  padding: 16px 4px 8px;
}
h1 {
  margin: 0 0 10px;
  font-size: 20px;
  word-break: break-word;
}
.types {
  margin-bottom: 4px;
}
.section {
  margin: 12px 0 20px;
}
.section h2 {
  display: flex;
  align-items: baseline;
  gap: 12px;
  margin: 0 4px 10px;
  font-size: 15px;
}
.all {
  color: var(--link);
  font-size: 13px;
  font-weight: 400;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: 0 4px;
}
.chip {
  display: inline-flex;
  align-items: baseline;
  gap: 4px;
  padding: 3px 12px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
}
.chip:hover {
  color: var(--text);
}
.chip.cat {
  background: var(--accent-soft);
  color: var(--accent);
}
.n {
  font-size: 11px;
  opacity: 0.6;
}
.blog-hits {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(260px, 100%), 1fr));
  gap: 10px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.hit {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: var(--card);
}
.hit:hover {
  border-color: color-mix(in srgb, var(--text) 25%, transparent);
}
.hit-text {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}
.hit-name,
.hit-sub {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hit-name {
  font-weight: 600;
  font-size: 14px;
}
.hit-sub {
  color: var(--text-3);
  font-size: 12px;
}
.more {
  display: flex;
  margin: 12px auto 0;
}
.rows {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(300px, 100%), 1fr));
  gap: 10px 16px;
  margin: 0 4px;
  padding: 0;
  list-style: none;
}
.hint {
  margin: 24px 4px;
  color: var(--text-3);
  font-size: 14px;
  text-align: center;
}
@media (max-width: 639px) {
  .head {
    padding: 12px 6px 4px;
  }
  .section h2,
  .chips,
  .rows {
    margin-left: 6px;
    margin-right: 6px;
  }
  .blog-hits {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
