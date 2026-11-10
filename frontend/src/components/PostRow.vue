<script setup lang="ts">
// One post as a compact row: thumbnail (or a coloured initial), title, blog.
// Opens the post over the current page.
import { computed, ref } from 'vue'
import { useRoute } from 'vue-router'
import { rememberPosts } from '../api'
import { hue, initial, relativeTime } from '../format'
import type { PostCard } from '../types'

const props = withDefaults(defineProps<{ post: PostCard; size?: number; replace?: boolean; note?: string }>(), {
  size: 56,
  replace: false,
  note: undefined,
})

const route = useRoute()
const broken = ref(false)
const link = computed(() => ({ path: route.path, query: { ...route.query, p: String(props.post.id) } }))
</script>

<template>
  <RouterLink :to="link" :replace="replace" class="row" @click="rememberPosts([post])">
    <img
      v-if="post.cover && !broken"
      :src="post.cover"
      alt=""
      loading="lazy"
      referrerpolicy="no-referrer"
      class="thumb"
      :style="{ width: `${size}px`, height: `${size}px` }"
      @error="broken = true"
    />
    <span
      v-else
      class="thumb thumb-text"
      :style="{ '--h': hue(post.blog.name), width: `${size}px`, height: `${size}px`, fontSize: `${size / 2.6}px` }"
      >{{ initial(post.title) }}</span
    >
    <span class="text">
      <span class="title">{{ post.title }}</span>
      <span class="meta">
        {{ post.blog.name }}
        <template v-if="note"> · {{ note }}</template>
        <template v-else-if="!post.undated"> · {{ relativeTime(post.published_at ?? post.sort_at) }}</template>
      </span>
    </span>
  </RouterLink>
</template>

<style scoped>
.row {
  display: flex;
  gap: 10px;
  align-items: center;
  min-width: 0;
}
.thumb {
  flex: none;
  border-radius: 8px;
  object-fit: cover;
  background: var(--surface);
}
.thumb-text {
  display: grid;
  place-items: center;
  background: hsl(var(--h) 60% 88%);
  color: hsl(var(--h) 45% 35%);
  font-weight: 700;
}
.text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.title {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
  font-size: 14px;
  line-height: 1.45;
  word-break: break-word;
}
.row:hover .title {
  color: var(--link);
}
.meta {
  overflow: hidden;
  color: var(--text-3);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
