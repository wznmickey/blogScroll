<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { allCategories, api, categoryName, meta, tagsInfo } from '../api'
import { hostOf } from '../format'
import { LIST_LABELS, searchList, type ListName } from '../library'
import type { BlogSummary, CategoryNode, PostCard, TagStat } from '../types'
import BlogAvatar from './BlogAvatar.vue'
import PostRow from './PostRow.vue'

const route = useRoute()
const router = useRouter()
const text = ref('')
const focused = ref(false)
const siteName = computed(() => meta.value?.site_name ?? '博客瀑布')
const logo = `${import.meta.env.BASE_URL}favicon.svg`
const hot = computed(() => tagsInfo.value?.popular_tags.slice(0, 24) ?? [])
const needle = computed(() => text.value.trim())
const showHot = computed(() => focused.value && !needle.value && hot.value.length > 0)

// As you type: matching categories and tags, bloggers, and posts from your
// own saves, likes and history.
const blogHits = ref<BlogSummary[]>([])
const tagHits = ref<TagStat[]>([])
const categoryHits = computed<CategoryNode[]>(() => {
  const n = needle.value.toLowerCase()
  if (!n) return []
  return allCategories(tagsInfo.value)
    .filter((c) => c.id !== 'other' && (c.label.toLowerCase().includes(n) || n.includes(c.label.toLowerCase())))
    .slice(0, 4)
})
const mineHits = computed<{ list: ListName; post: PostCard }[]>(() => {
  if (!needle.value) return []
  const seen = new Set<number>()
  const out: { list: ListName; post: PostCard }[] = []
  for (const list of ['saves', 'likes', 'history', 'seen'] as const) {
    for (const e of searchList(list, needle.value)) {
      if (seen.has(e.post.id)) continue
      seen.add(e.post.id)
      out.push({ list, post: e.post })
    }
  }
  return out.slice(0, 3)
})
const showSuggest = computed(
  () =>
    focused.value &&
    !!needle.value &&
    (blogHits.value.length + tagHits.value.length + categoryHits.value.length + mineHits.value.length > 0),
)

let suggestTimer: ReturnType<typeof setTimeout> | undefined
watch(needle, (q) => {
  clearTimeout(suggestTimer)
  if (!q) {
    blogHits.value = []
    tagHits.value = []
    return
  }
  suggestTimer = setTimeout(async () => {
    const [blogs, tags] = await Promise.allSettled([api.blogs({ q, limit: 4 }), api.searchTags(q, 6)])
    if (needle.value !== q) return
    blogHits.value = blogs.status === 'fulfilled' ? blogs.value.items : []
    tagHits.value = tags.status === 'fulfilled' ? tags.value : []
  }, 200)
})

watch(
  () => [route.name, route.query.q],
  () => (text.value = route.name === 'search' && typeof route.query.q === 'string' ? route.query.q : ''),
  { immediate: true },
)

function blur() {
  ;(document.activeElement as HTMLElement | null)?.blur()
}

function go(to: Parameters<typeof router.push>[0]) {
  router.push(to)
  blur()
}

function submit() {
  const q = needle.value
  go(q ? { path: '/search', query: { q } } : { path: '/search' })
}
</script>

<template>
  <header class="topbar">
    <div class="container inner">
      <RouterLink to="/" class="logo">
        <img :src="logo" alt="" width="28" height="28" />
        <span>{{ siteName }}</span>
      </RouterLink>
      <form class="search" role="search" @submit.prevent="submit">
        <label class="visually-hidden" for="search-input">搜索</label>
        <input
          id="search-input"
          v-model="text"
          type="search"
          placeholder="搜索文章、博主、标签、收藏"
          enterkeyhint="search"
          autocomplete="off"
          @focus="focused = true"
          @blur="focused = false"
          @keydown.esc="blur"
        />
        <button type="submit" aria-label="搜索">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-3.5-3.5" stroke-linecap="round" />
          </svg>
        </button>
        <!-- mousedown.prevent keeps focus in the input until the click lands. -->
        <div v-if="showSuggest" class="hot suggest" @mousedown.prevent>
          <template v-if="categoryHits.length || tagHits.length">
            <p>分类与标签</p>
            <div class="hot-list">
              <button
                v-for="c in categoryHits"
                :key="c.id"
                type="button"
                class="cat"
                @click="go({ path: '/', query: { c: `cat:${c.id}` } })"
              >
                {{ categoryName(c.id) }}
              </button>
              <button
                v-for="t in tagHits"
                :key="t.name"
                type="button"
                @click="go({ path: '/', query: { c: `tag:${t.name}` } })"
              >
                #{{ t.name }}
              </button>
            </div>
          </template>
          <template v-if="blogHits.length">
            <p>博主</p>
            <button v-for="b in blogHits" :key="b.id" type="button" class="sug" @click="go(`/blog/${b.id}`)">
              <BlogAvatar :name="b.name" :icon="b.icon" :size="28" />
              <span class="sug-text">
                <span class="sug-name">{{ b.name }}</span>
                <span class="sug-sub">{{ b.description || hostOf(b.homepage) }}</span>
              </span>
            </button>
          </template>
          <template v-if="mineHits.length">
            <p>我的收藏、喜欢、历史和刷到过</p>
            <div v-for="m in mineHits" :key="m.post.id" class="mine" @click="blur">
              <PostRow :post="m.post" :size="36" :note="LIST_LABELS[m.list]" />
            </div>
          </template>
          <button type="button" class="sug all" @click="submit">搜索“{{ needle }}”的全部结果 ↵</button>
        </div>
        <div v-if="showHot" class="hot" @mousedown.prevent>
          <p>热门标签</p>
          <div class="hot-list">
            <button v-for="c in hot" :key="c.name" type="button" @click="go({ path: '/', query: { c: `tag:${c.name}` } })">
              #{{ c.name }}
            </button>
          </div>
        </div>
      </form>
      <nav class="nav">
        <RouterLink to="/" class="desktop-only" exact-active-class="active">首页</RouterLink>
        <RouterLink to="/blogs" active-class="active">博客</RouterLink>
        <RouterLink to="/me" active-class="active">我的</RouterLink>
        <RouterLink to="/about" active-class="active">关于</RouterLink>
      </nav>
    </div>
  </header>
</template>

<style scoped>
.topbar {
  position: sticky;
  top: 0;
  z-index: 20;
  background: color-mix(in srgb, var(--bg) 88%, transparent);
  backdrop-filter: saturate(180%) blur(16px);
  -webkit-backdrop-filter: saturate(180%) blur(16px);
}
.inner {
  display: flex;
  align-items: center;
  gap: 24px;
  height: var(--header-h);
}
.logo {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
  font-size: 18px;
  font-weight: 700;
  color: var(--accent);
  letter-spacing: 0.02em;
}
.search {
  position: relative;
  flex: 1;
  max-width: 520px;
  margin: 0 auto;
}
.search input {
  width: 100%;
  height: 40px;
  padding: 0 44px 0 18px;
  border: 0;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-size: 14px;
  outline: none;
  -webkit-appearance: none;
  appearance: none;
}
.search input::placeholder {
  color: var(--text-3);
}
.search input:focus {
  box-shadow: 0 0 0 2px var(--accent-soft);
}
.search > button {
  position: absolute;
  right: 6px;
  top: 50%;
  transform: translateY(-50%);
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  color: var(--text-2);
}
.nav {
  display: flex;
  flex: none;
  gap: 20px;
  font-size: 14px;
  color: var(--text-2);
}
.nav a:hover,
.nav a.active {
  color: var(--text);
}
.nav a.active {
  font-weight: 600;
}
.hot {
  position: absolute;
  top: calc(100% + 8px);
  left: 0;
  right: 0;
  padding: 12px 14px 14px;
  border-radius: 16px;
  background: var(--bg);
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.16);
  border: 1px solid var(--line);
}
.hot p {
  margin: 0 0 8px;
  color: var(--text-3);
  font-size: 12px;
}
.hot-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 8px;
}
.hot-list button {
  padding: 2px 10px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
}
.hot-list button:hover {
  color: var(--link);
}
.hot-list .cat {
  background: var(--accent-soft);
  color: var(--accent);
}
.hot {
  max-height: min(70vh, 560px);
  overflow-y: auto;
  overscroll-behavior: contain;
}
.suggest .hot-list {
  padding: 0 6px;
}
.suggest p:not(:first-child) {
  margin-top: 10px;
}
.mine {
  padding: 4px 6px;
  border-radius: 10px;
}
.mine:hover {
  background: var(--surface);
}
.suggest {
  padding: 10px 8px 8px;
}
.suggest p {
  padding: 0 6px;
}
.sug {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 6px;
  border-radius: 10px;
  text-align: left;
}
.sug:hover {
  background: var(--surface);
}
.sug-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.sug-name,
.sug-sub {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sug-name {
  font-size: 14px;
}
.sug-sub {
  color: var(--text-3);
  font-size: 12px;
}
.sug.all {
  justify-content: center;
  margin-top: 4px;
  color: var(--link);
  font-size: 13px;
}
@media (max-width: 639px) {
  .inner {
    gap: 10px;
    padding: 0 12px;
  }
  .logo span {
    display: none;
  }
  .search input {
    height: 34px;
    font-size: 13px;
  }
  .nav {
    gap: 12px;
  }
  .desktop-only {
    display: none;
  }
}
</style>
