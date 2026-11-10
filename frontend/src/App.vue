<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { loadMeta, loadTags } from './api'
import AppHeader from './components/AppHeader.vue'
import PostDetail from './components/PostDetail.vue'
import ReaderPane from './components/ReaderPane.vue'

const route = useRoute()
const router = useRouter()
loadMeta()
loadTags()

const openPost = computed(() => {
  const p = Number(route.query.p)
  return Number.isInteger(p) && p > 0 ? p : null
})

function closePost() {
  // Opened from a card: go back so the browser history stays clean.
  // Opened from a shared link: there is nothing to go back to.
  if (window.history.state?.back) {
    router.back()
  } else {
    const { p: _, ...rest } = route.query
    router.replace({ query: rest })
  }
}
</script>

<template>
  <AppHeader />
  <main>
    <RouterView v-slot="{ Component }">
      <KeepAlive :include="['HomeView', 'BlogsView']">
        <component :is="Component" />
      </KeepAlive>
    </RouterView>
  </main>
  <PostDetail v-if="openPost" :id="openPost" :key="openPost" @close="closePost" />
  <ReaderPane />
</template>
