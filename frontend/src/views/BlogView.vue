<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api'
import BlogAvatar from '../components/BlogAvatar.vue'
import Waterfall from '../components/Waterfall.vue'
import { hostOf, relativeTime } from '../format'
import type { BlogDetail } from '../types'

const route = useRoute()
const id = computed(() => Number(route.params.id))
const blog = ref<BlogDetail | null>(null)
const missing = ref(false)

watch(
  id,
  async (blogId) => {
    blog.value = null
    missing.value = false
    try {
      blog.value = await api.blog(blogId)
      document.title = blog.value.name
    } catch {
      missing.value = true
    }
  },
  { immediate: true },
)

const load = (cursor: string | null) => api.posts({ mode: 'latest', blog: id.value, cursor })

/** "Hexo 7.3.0" → "Hexo" */
const engine = computed(() => blog.value?.generator?.split(/\s+/)[0] ?? null)

const SOCIAL: Record<string, string> = {
  github: 'GitHub',
  gitee: 'Gitee',
  bilibili: 'B站',
  weibo: '微博',
  zhihu: '知乎',
  x: 'X',
  telegram: 'Telegram',
  instagram: 'Instagram',
  youtube: 'YouTube',
  douban: '豆瓣',
  xiaohongshu: '小红书',
  link: '个人主页',
}
const socialLabel = (kind: string) => SOCIAL[kind] ?? kind
</script>

<template>
  <div class="container">
    <section v-if="blog" class="profile">
      <BlogAvatar :name="blog.name" :icon="blog.icon" :size="88" />
      <div class="info">
        <h1>{{ blog.name }}</h1>
        <a :href="blog.homepage" target="_blank" rel="noopener" class="home">{{ hostOf(blog.homepage) }} ↗</a>
        <p v-if="blog.description" class="desc">{{ blog.description }}</p>
        <p class="chips">
          <span v-for="d in blog.directories" :key="d.directory" class="chip source">收录于{{ d.name }}</span>
          <RouterLink
            v-for="t in blog.tags"
            :key="t.directory + t.tag"
            :to="{ path: '/', query: { tag: t.tag } }"
            class="chip link"
            >{{ t.label }}</RouterLink
          >
          <span v-if="blog.full_text" class="chip">作者已授权全文</span>
          <span v-if="engine" class="chip">{{ engine }}</span>
          <span v-if="blog.comment_system" class="chip">评论 · {{ blog.comment_system }}</span>
        </p>
        <p v-if="blog.social.length" class="social">
          <a v-for="s in blog.social" :key="s.url" :href="s.url" target="_blank" rel="noopener me">{{
            socialLabel(s.kind)
          }}</a>
        </p>
        <p class="stats">
          <span><b>{{ blog.post_count }}</b> 篇文章</span>
          <span v-if="blog.last_post_at">最近更新 {{ relativeTime(blog.last_post_at) }}</span>
          <span v-if="!blog.has_feed">暂未找到 RSS</span>
        </p>
      </div>
    </section>
    <section v-if="blog?.about_text" class="about">
      <h2>关于</h2>
      <p>{{ blog.about_text }}</p>
      <a v-if="blog.about_url" :href="blog.about_url" target="_blank" rel="noopener">阅读完整介绍 ↗</a>
    </section>
    <p v-else-if="missing" class="missing">找不到这个博客</p>

    <Waterfall v-if="!missing" :load="load" :reset-key="String(id)">
      <template #empty><p class="missing">暂时还没有抓取到这个博客的文章</p></template>
    </Waterfall>
  </div>
</template>

<style scoped>
.profile {
  display: flex;
  align-items: center;
  gap: 28px;
  max-width: 880px;
  margin: 24px auto 32px;
  padding: 0 8px;
}
.info {
  min-width: 0;
}
h1 {
  margin: 0;
  font-size: 24px;
  line-height: 1.3;
  word-break: break-word;
}
.home {
  color: var(--link);
  font-size: 14px;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 10px 0 0;
}
.chip {
  padding: 2px 10px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 12px;
}
.desc {
  margin: 6px 0 0;
  color: var(--text-2);
  font-size: 14px;
}
.social {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  margin: 10px 0 0;
  font-size: 13px;
}
.social a {
  color: var(--link);
}
.about {
  max-width: 880px;
  margin: -12px auto 28px;
  padding: 14px 18px;
  border-radius: 14px;
  background: var(--surface);
}
.about h2 {
  margin: 0 0 4px;
  font-size: 14px;
}
.about p {
  margin: 0;
  color: var(--text-2);
  font-size: 14px;
  line-height: 1.7;
}
.about a {
  display: inline-block;
  margin-top: 6px;
  color: var(--link);
  font-size: 13px;
}
.chip.link:hover {
  color: var(--text);
}
.chip.source {
  background: var(--accent-soft);
  color: var(--accent);
}
.stats {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  margin: 10px 0 0;
  color: var(--text-2);
  font-size: 13px;
}
.stats b {
  color: var(--text);
}
.missing {
  margin: 48px 0;
  text-align: center;
  color: var(--text-2);
}
@media (max-width: 639px) {
  .profile {
    gap: 16px;
    margin: 16px 6px 20px;
  }
  .profile :deep(.avatar) {
    width: 64px !important;
    height: 64px !important;
  }
  h1 {
    font-size: 20px;
  }
}
</style>
