<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { api, cachedPost } from '../api'
import { fullDate, hostOf } from '../format'
import type { PostCard, PostDetail } from '../types'
import BlogAvatar from './BlogAvatar.vue'
import TextCover from './TextCover.vue'

const props = defineProps<{ id: number }>()
const emit = defineEmits<{ close: [] }>()

const card = ref<PostCard | undefined>(cachedPost(props.id))
const detail = ref<PostDetail | null>(null)
const error = ref(false)
const coverFailed = ref(false)
const post = computed<PostCard | PostDetail | undefined>(() => detail.value ?? card.value)
const hasCover = computed(() => !!post.value?.cover && !coverFailed.value)

onMounted(async () => {
  document.addEventListener('keydown', onKey)
  document.documentElement.style.overflow = 'hidden'
  try {
    detail.value = await api.post(props.id)
  } catch {
    error.value = true
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('keydown', onKey)
  document.documentElement.style.overflow = ''
})

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="sheet" :class="{ 'no-media': !hasCover }" role="dialog" aria-modal="true" :aria-label="post?.title">
      <button class="close" aria-label="关闭" @click="emit('close')">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M6 6l12 12M18 6 6 18" />
        </svg>
      </button>

      <template v-if="post">
        <header class="mobile-bar">
          <button class="back" aria-label="返回" @click="emit('close')">
            <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <path d="m15 5-7 7 7 7" />
            </svg>
          </button>
          <RouterLink :to="`/blog/${post.blog.id}`" class="who">
            <BlogAvatar :name="post.blog.name" :icon="post.blog.icon" :size="30" />
            <span>{{ post.blog.name }}</span>
          </RouterLink>
          <a :href="post.blog.homepage" target="_blank" rel="noopener" class="btn btn-ghost visit">访问博客</a>
        </header>

        <div class="media">
          <img
            v-if="hasCover"
            :src="post.cover!"
            alt=""
            referrerpolicy="no-referrer"
            @error="coverFailed = true"
          />
          <TextCover v-else class="media-text" :post="post" :ratio="1.33" large />
        </div>

        <section class="panel">
          <header class="panel-head">
            <RouterLink :to="`/blog/${post.blog.id}`" class="who">
              <BlogAvatar :name="post.blog.name" :icon="post.blog.icon" :size="36" />
              <span>{{ post.blog.name }}</span>
            </RouterLink>
            <a :href="post.blog.homepage" target="_blank" rel="noopener" class="btn btn-ghost visit">访问博客</a>
          </header>

          <div class="scroll">
            <h1 class="title">{{ post.title }}</h1>
            <div v-if="detail?.content_html" class="content" v-html="detail.content_html" />
            <template v-else>
              <p v-if="post.excerpt" class="excerpt">{{ post.excerpt }}</p>
              <p class="note">
                以上为 RSS 摘要。为尊重作者，完整内容请到原博客阅读。
              </p>
            </template>
            <p v-if="post.tags.length" class="tags">
              <span v-for="t in post.tags" :key="t">#{{ t }}</span>
            </p>
            <p class="date">
              {{ fullDate(post.published_at ?? post.sort_at) }}
              <template v-if="post.author && post.author !== post.blog.name"> · {{ post.author }}</template>
            </p>
            <p v-if="error" class="note">详情加载失败，可直接阅读原文。</p>
          </div>

          <footer class="actions">
            <span class="host">{{ hostOf(post.url) }}</span>
            <a :href="post.url" target="_blank" rel="noopener" class="btn btn-primary read">阅读原文 ↗</a>
          </footer>
        </section>
      </template>
      <div v-else-if="error" class="missing">
        <p>找不到这篇文章</p>
        <button class="btn btn-ghost" @click="emit('close')">返回</button>
      </div>
      <div v-else class="missing"><span class="spinner" /></div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: grid;
  place-items: center;
  padding: 24px;
  background: var(--overlay);
  animation: fade 0.18s ease-out;
}
.sheet {
  position: relative;
  display: flex;
  width: min(1120px, 100%);
  height: min(860px, 100%);
  overflow: hidden;
  border-radius: 20px;
  background: var(--bg);
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
  animation: pop 0.22s cubic-bezier(0.2, 0.9, 0.3, 1.1);
}
.sheet.no-media {
  width: min(1000px, 100%);
}
.close {
  position: absolute;
  top: 14px;
  left: 14px;
  z-index: 2;
  display: grid;
  place-items: center;
  width: 36px;
  height: 36px;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.45);
  color: #fff;
}
.mobile-bar {
  display: none;
}
.media {
  flex: 1 1 58%;
  min-width: 0;
  display: grid;
  place-items: center;
  background: var(--surface);
}
.media img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.media-text {
  width: 100%;
  height: 100%;
  border-radius: 0;
}
.panel {
  flex: 0 0 420px;
  display: flex;
  flex-direction: column;
  min-width: 0;
  border-left: 1px solid var(--line);
}
.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 20px 24px;
  border-bottom: 1px solid var(--line);
}
.who {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  font-weight: 600;
}
.who span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.visit {
  height: 32px;
  font-size: 13px;
}
.scroll {
  flex: 1;
  overflow-y: auto;
  padding: 20px 24px 24px;
  overscroll-behavior: contain;
}
.title {
  margin: 0 0 12px;
  font-size: 19px;
  line-height: 1.45;
  font-weight: 700;
  word-break: break-word;
}
.excerpt {
  margin: 0 0 12px;
  white-space: pre-line;
  word-break: break-word;
}
.note {
  margin: 12px 0;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  margin: 12px 0 0;
  color: var(--link);
  font-size: 14px;
}
.date {
  margin: 12px 0 0;
  color: var(--text-3);
  font-size: 13px;
}
.actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 24px calc(14px + var(--safe-b));
  border-top: 1px solid var(--line);
}
.host {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-3);
  font-size: 13px;
}
.read {
  height: 40px;
  padding: 0 22px;
}
.missing {
  display: grid;
  place-items: center;
  gap: 12px;
  width: 100%;
  color: var(--text-2);
}
.spinner {
  width: 24px;
  height: 24px;
  border: 2px solid var(--line);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
.content {
  word-break: break-word;
  line-height: 1.75;
}
.content :deep(img) {
  height: auto;
  border-radius: 8px;
  margin: 8px 0;
}
.content :deep(pre) {
  overflow-x: auto;
  padding: 12px;
  border-radius: 8px;
  background: var(--surface);
  font-size: 13px;
}
.content :deep(a) {
  color: var(--link);
}
.content :deep(blockquote) {
  margin: 8px 0;
  padding-left: 12px;
  border-left: 3px solid var(--line);
  color: var(--text-2);
}

@keyframes fade {
  from {
    opacity: 0;
  }
}
@keyframes pop {
  from {
    opacity: 0;
    transform: scale(0.96);
  }
}
@keyframes slide {
  from {
    transform: translateY(24px);
    opacity: 0;
  }
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* Text-only posts on desktop: just the text, no empty media pane. */
@media (min-width: 768px) {
  .no-media .media {
    flex-basis: 45%;
  }
}

/* Phones and small tablets: full-screen page instead of a modal. */
@media (max-width: 767px) {
  .overlay {
    display: block;
    padding: 0;
    background: var(--bg);
    animation: none;
  }
  .sheet,
  .sheet.no-media {
    flex-direction: column;
    width: 100%;
    height: 100%;
    border-radius: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    box-shadow: none;
    animation: slide 0.22s ease-out;
  }
  .close,
  .panel-head {
    display: none;
  }
  .mobile-bar {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
    height: 52px;
    padding: 0 12px 0 4px;
    background: var(--bg);
    border-bottom: 1px solid var(--line);
  }
  .back {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
  }
  .mobile-bar .who {
    flex: 1;
  }
  .media {
    flex: none;
    max-height: 72vh;
  }
  .media img {
    height: auto;
    max-height: 72vh;
  }
  .media-text {
    min-height: 0;
    aspect-ratio: 4 / 3;
    padding: 40px 28px;
  }
  .media-text :deep(p) {
    font-size: 18px;
  }
  .panel {
    flex: 1 0 auto;
    border-left: 0;
  }
  .scroll {
    overflow: visible;
    padding: 16px 16px 24px;
  }
  .actions {
    position: sticky;
    bottom: 0;
    background: var(--bg);
    padding: 10px 16px calc(10px + var(--safe-b));
  }
  .read {
    flex: 1;
    max-width: 260px;
  }
}
</style>
