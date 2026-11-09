<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { meta } from '../api'

const route = useRoute()
const router = useRouter()
const text = ref('')
const siteName = computed(() => meta.value?.site_name ?? '博客漫游')

watch(
  () => route.query.q,
  (q) => (text.value = typeof q === 'string' ? q : ''),
  { immediate: true },
)

function submit() {
  const q = text.value.trim()
  router.push(q ? { path: '/', query: { q } } : { path: '/' })
  ;(document.activeElement as HTMLElement | null)?.blur()
}
</script>

<template>
  <header class="topbar">
    <div class="container inner">
      <RouterLink to="/" class="logo">
        <img src="/favicon.svg" alt="" width="28" height="28" />
        <span>{{ siteName }}</span>
      </RouterLink>
      <form class="search" role="search" @submit.prevent="submit">
        <label class="visually-hidden" for="search-input">搜索</label>
        <input id="search-input" v-model="text" type="search" placeholder="搜索文章或博客" enterkeyhint="search" />
        <button type="submit" aria-label="搜索">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-3.5-3.5" stroke-linecap="round" />
          </svg>
        </button>
      </form>
      <nav class="nav">
        <RouterLink to="/about">关于</RouterLink>
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
.search button {
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
  flex: none;
  font-size: 14px;
  color: var(--text-2);
}
.nav a:hover,
.nav a.router-link-active {
  color: var(--text);
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
}
</style>
