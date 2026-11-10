<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api, meta, tagsInfo } from '../api'
import { hostOf } from '../format'
import type { BlogSummary } from '../types'
import BlogAvatar from './BlogAvatar.vue'

const route = useRoute()
const router = useRouter()
const text = ref('')
const focused = ref(false)
const siteName = computed(() => meta.value?.site_name ?? '博客漫游')
const logo = `${import.meta.env.BASE_URL}favicon.svg`
const hot = computed(() => tagsInfo.value?.categories.slice(0, 24) ?? [])
const showHot = computed(() => focused.value && !text.value.trim() && hot.value.length > 0)

// Bloggers matching what is typed, as suggestions under the box.
const suggestions = ref<BlogSummary[]>([])
const showSuggest = computed(() => focused.value && !!text.value.trim() && suggestions.value.length > 0)
let suggestTimer: ReturnType<typeof setTimeout> | undefined
watch(text, (v) => {
  clearTimeout(suggestTimer)
  const q = v.trim()
  if (!q) {
    suggestions.value = []
    return
  }
  suggestTimer = setTimeout(async () => {
    try {
      const page = await api.blogs({ q, limit: 5 })
      if (text.value.trim() === q) suggestions.value = page.items
    } catch {
      suggestions.value = []
    }
  }, 200)
})

function openBlog(id: number) {
  router.push(`/blog/${id}`)
  blur()
}

watch(
  () => route.query.q,
  (q) => (text.value = typeof q === 'string' ? q : ''),
  { immediate: true },
)

function blur() {
  ;(document.activeElement as HTMLElement | null)?.blur()
}

function submit() {
  const q = text.value.trim()
  router.push(q ? { path: '/', query: { q } } : { path: '/' })
  blur()
}

function pick(category: string) {
  router.push({ path: '/', query: { category } })
  blur()
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
          placeholder="搜索文章或博主"
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
          <p>博主</p>
          <button v-for="b in suggestions" :key="b.id" type="button" class="sug" @click="openBlog(b.id)">
            <BlogAvatar :name="b.name" :icon="b.icon" :size="28" />
            <span class="sug-text">
              <span class="sug-name">{{ b.name }}</span>
              <span class="sug-sub">{{ b.description || hostOf(b.homepage) }}</span>
            </span>
          </button>
          <button type="button" class="sug all" @click="submit">搜索“{{ text.trim() }}”的文章和博主 ↵</button>
        </div>
        <div v-if="showHot" class="hot" @mousedown.prevent>
          <p>热门标签</p>
          <div class="hot-list">
            <button v-for="c in hot" :key="c.name" type="button" @click="pick(c.name)">#{{ c.name }}</button>
          </div>
        </div>
      </form>
      <nav class="nav">
        <RouterLink to="/" class="desktop-only" exact-active-class="active">发现</RouterLink>
        <RouterLink to="/blogs" active-class="active">博客</RouterLink>
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
