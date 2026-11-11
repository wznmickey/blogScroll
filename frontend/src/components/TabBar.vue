<script setup lang="ts">
// Phones: the main places along the bottom, within reach of a thumb (the
// header keeps just the logo and the search). Tapping 首页 while on it goes
// back to the top.
import { useRoute } from 'vue-router'

const route = useRoute()

const TABS = [
  { to: '/', label: '首页', icon: 'M4 11 12 4l8 7v8a1 1 0 0 1-1 1h-4v-6h-6v6H5a1 1 0 0 1-1-1z', exact: true },
  { to: '/trending', label: '热门', icon: 'M12 3s6 5 6 11a6 6 0 0 1-12 0c0-2.7 1.3-4.7 2.5-6 0 2.3 1.2 3.5 2.5 3.5 0-3.5-1-6 1-8.5z', exact: false },
  { to: '/blogs', label: '博客', icon: 'M5 4h10l4 4v12H5zM14 4v5h5M8 13h8M8 17h5', exact: false },
  { to: '/me', label: '我的', icon: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 20a8 8 0 0 1 16 0', exact: false },
  {
    to: '/settings',
    label: '设置',
    icon: 'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19 12a7 7 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7 7 0 0 0-2-1.2L14 3h-4l-.5 2.6a7 7 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6a7 7 0 0 0 0 2.4l-2 1.6 2 3.4 2.4-1a7 7 0 0 0 2 1.2L10 21h4l.5-2.6a7 7 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6c.1-.4.1-.8.1-1.2z',
    exact: false,
  },
]

const active = (t: (typeof TABS)[number]) => (t.exact ? route.path === t.to : route.path.startsWith(t.to))

function tap(t: (typeof TABS)[number]) {
  if (t.to === '/' && route.path === '/') window.scrollTo({ top: 0, behavior: 'smooth' })
}
</script>

<template>
  <nav class="tabbar" aria-label="主要页面">
    <RouterLink
      v-for="t in TABS"
      :key="t.to"
      :to="t.to"
      class="tab"
      :class="{ active: active(t) }"
      :aria-current="active(t) ? 'page' : undefined"
      @click="tap(t)"
    >
      <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path :d="t.icon" />
      </svg>
      <span>{{ t.label }}</span>
    </RouterLink>
  </nav>
</template>

<style scoped>
.tabbar {
  display: none;
}
@media (max-width: 639px) {
  .tabbar {
    position: fixed;
    right: 0;
    bottom: 0;
    left: 0;
    z-index: 40;
    display: flex;
    height: calc(var(--tabbar-h) + env(safe-area-inset-bottom));
    padding-bottom: env(safe-area-inset-bottom);
    border-top: 1px solid var(--line);
    background: color-mix(in srgb, var(--bg) 92%, transparent);
    backdrop-filter: saturate(180%) blur(16px);
    -webkit-backdrop-filter: saturate(180%) blur(16px);
  }
  .tab {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    color: var(--text-3);
    font-size: 11px;
  }
  .tab.active {
    color: var(--accent);
    font-weight: 600;
  }
}
</style>
