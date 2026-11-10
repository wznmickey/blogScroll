<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute } from 'vue-router'
import { COVER_DEFAULT, clampCover, relativeTime, textCoverRatio } from '../format'
import type { PostCard } from '../types'
import BlogAvatar from './BlogAvatar.vue'
import TextCover from './TextCover.vue'

const props = defineProps<{ post: PostCard }>()
const emit = defineEmits<{ aspect: [ratio: number] }>()

const route = useRoute()
const failed = ref(false)
const ratio = ref<number | null>(null)

const showImage = computed(() => !!props.post.cover && !failed.value)
const detailLink = computed(() => ({ path: route.path, query: { ...route.query, p: String(props.post.id) } }))

function onLoad(e: Event) {
  const img = e.target as HTMLImageElement
  // Tiny images are icons or tracking pixels that slipped through.
  if (img.naturalWidth < 120 || img.naturalHeight < 80) {
    failed.value = true
    return
  }
  ratio.value = clampCover(img.naturalHeight / img.naturalWidth)
  emit('aspect', ratio.value)
}
</script>

<template>
  <article class="card">
    <RouterLink :to="detailLink" class="body">
      <div v-if="showImage" class="cover" :style="{ aspectRatio: `1 / ${ratio ?? COVER_DEFAULT}` }">
        <img
          :src="post.cover!"
          alt=""
          loading="lazy"
          decoding="async"
          referrerpolicy="no-referrer"
          @load="onLoad"
          @error="failed = true"
        />
      </div>
      <TextCover v-else :post="post" :ratio="textCoverRatio(post.id)" />
      <h3 class="title">{{ post.title }}</h3>
    </RouterLink>
    <footer class="foot">
      <RouterLink :to="`/blog/${post.blog.id}`" class="author">
        <BlogAvatar :name="post.blog.name" :icon="post.blog.icon" :size="20" />
        <span class="name">{{ post.blog.name }}</span>
      </RouterLink>
      <span v-if="post.comment_count" class="count" :title="`${post.comment_count} 条评论`">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
          <path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z" stroke-linejoin="round" />
        </svg>
        {{ post.comment_count }}
      </span>
      <time v-else-if="!post.undated" class="time" :datetime="new Date(post.sort_at * 1000).toISOString()">{{
        relativeTime(post.sort_at)
      }}</time>
    </footer>
  </article>
</template>

<style scoped>
.card {
  background: var(--card);
  border-radius: 8px;
  overflow: hidden;
}
.body {
  display: block;
}
.cover {
  overflow: hidden;
  border-radius: var(--radius-cover);
  background: var(--surface);
}
.cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  transition: transform 0.3s ease;
}
@media (hover: hover) {
  .body:hover .cover img {
    transform: scale(1.03);
  }
}
.title {
  margin: 0;
  padding: 10px 4px 4px;
  font-size: 14px;
  font-weight: 500;
  line-height: 1.45;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
  word-break: break-word;
}
.foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 2px 4px 6px;
  font-size: 12px;
  color: var(--text-2);
}
.author {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.author:hover .name {
  color: var(--text);
}
.name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.time {
  flex: none;
  color: var(--text-3);
}
.count {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  color: var(--text-2);
}
@media (max-width: 639px) {
  .title {
    padding: 8px 8px 4px;
  }
  .foot {
    padding: 2px 8px 10px;
  }
}
</style>
