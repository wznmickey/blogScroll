<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api'
import BlogAvatar from '../components/BlogAvatar.vue'
import FollowButton from '../components/FollowButton.vue'
import Waterfall from '../components/Waterfall.vue'
import SocialLinks from '../components/SocialLinks.vue'
import { hostOf, relativeTime } from '../format'
import { recordShown } from '../library'
import { openInReader, readerIntent, readerIntentEnd } from '../reader'
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

const aboutOpen = ref(false)
watch(id, () => (aboutOpen.value = false))

function openBeside(e: MouseEvent, url: string, title: string) {
  if (blog.value && openInReader({ url, title, blog: blog.value }, e)) e.preventDefault()
}

/** Resting on a link to the blog's site loads it in the (hidden) pane. */
function ahead(url: string, title: string, delay?: number) {
  if (blog.value) readerIntent({ url, title, blog: blog.value }, delay)
}

/** "Hexo 7.3.0" → "Hexo" */
const engine = computed(() => blog.value?.generator?.split(/\s+/)[0] ?? null)
</script>

<template>
  <div class="container">
    <section v-if="blog" class="profile">
      <BlogAvatar :name="blog.name" :icon="blog.icon" :size="88" />
      <div class="info">
        <div class="title-row">
          <h1>{{ blog.name }}</h1>
          <FollowButton :blog-id="blog.id" />
        </div>
        <a
          :href="blog.homepage"
          target="_blank"
          rel="noopener"
          class="home"
          @pointerenter="ahead(blog.homepage, blog.name)"
          @pointerleave="readerIntentEnd"
          @focus="ahead(blog.homepage, blog.name)"
          @pointerdown="ahead(blog.homepage, blog.name, 0)"
          @click="openBeside($event, blog.homepage, blog.name)"
          >{{ hostOf(blog.homepage) }} ↗</a
        >
        <p v-if="blog.description" class="desc">{{ blog.description }}</p>
        <p class="chips">
          <span v-if="blog.kind && blog.kind.id !== 'blog'" class="chip source">{{ blog.kind.label }}</span>
          <span v-for="d in blog.directories" :key="d.directory" class="chip source">收录于{{ d.name }}</span>
          <RouterLink
            v-for="c in blog.categories ?? []"
            :key="c.id"
            :to="{ path: '/', query: { c: `cat:${c.id}` } }"
            class="chip link topic"
            :title="c.share ? `约 ${Math.round(c.share * 100)}% 的文章` : undefined"
            >{{ c.label }}</RouterLink
          >
          <RouterLink
            v-for="t in blog.tags"
            :key="t.directory + t.tag"
            :to="{ path: '/', query: { dirtag: t.tag } }"
            class="chip link"
            >{{ t.label }}</RouterLink
          >
          <span v-if="blog.full_text" class="chip">作者已授权全文</span>
          <span v-if="engine" class="chip">{{ engine }}</span>
          <span v-if="blog.comment_system" class="chip">评论 · {{ blog.comment_system }}</span>
        </p>
        <p v-if="blog.top_tags?.length" class="top-tags">
          <RouterLink v-for="t in blog.top_tags" :key="t" :to="{ path: '/', query: { c: `tag:${t}` } }">#{{ t }}</RouterLink>
        </p>
        <SocialLinks v-if="blog?.social.length" :links="blog.social" />
        <p class="stats">
          <span><b>{{ blog.post_count }}</b> 篇文章</span>
          <span v-if="blog.last_post_at">最近更新 {{ relativeTime(blog.last_post_at) }}</span>
          <span v-if="!blog.has_feed">暂未找到 RSS</span>
          <RouterLink :to="{ path: '/me', query: { tab: 'blog', blog: blog.id } }" class="mine">这是你的博客？查看抓取情况</RouterLink>
        </p>
      </div>
    </section>
    <section v-if="blog?.about_html || blog?.about_text" class="about">
      <h2>关于</h2>
      <!-- The whole page, for authors who asked for it (sanitised on the server). -->
      <template v-if="blog.about_html">
        <div class="about-html" :class="{ clamped: !aboutOpen }" v-html="blog.about_html" />
        <button type="button" class="more" @click="aboutOpen = !aboutOpen">{{ aboutOpen ? '收起' : '展开全部' }}</button>
      </template>
      <p v-else>{{ blog.about_text }}</p>
      <a
        v-if="blog.about_url"
        :href="blog.about_url"
        target="_blank"
        rel="noopener"
        @pointerenter="ahead(blog.about_url, `关于 · ${blog.name}`)"
        @pointerleave="readerIntentEnd"
        @focus="ahead(blog.about_url, `关于 · ${blog.name}`)"
        @pointerdown="ahead(blog.about_url, `关于 · ${blog.name}`, 0)"
        @click="openBeside($event, blog.about_url, `关于 · ${blog.name}`)"
        >阅读完整介绍 ↗</a
      >
    </section>
    <p v-else-if="missing" class="missing">找不到这个博客</p>

    <Waterfall
      v-if="!missing"
      :load="load"
      :reset-key="String(id)"
      @impression="(p) => recordShown(p, blog?.name ?? '博客页')"
    >
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
.title-row {
  display: flex;
  align-items: center;
  gap: 14px;
}
h1 {
  margin: 0;
  font-size: 24px;
  line-height: 1.3;
  word-break: break-word;
}
.chip.topic {
  background: color-mix(in srgb, var(--link) 12%, transparent);
  color: var(--link);
}
.top-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  margin: 8px 0 0;
  color: var(--link);
  font-size: 13px;
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
  /* An author's own text keeps its line breaks. */
  white-space: pre-line;
}
.about-html {
  color: var(--text-2);
  font-size: 14px;
  line-height: 1.75;
  word-break: break-word;
}
.about-html.clamped {
  max-height: 16em;
  overflow: hidden;
  mask-image: linear-gradient(to bottom, #000 70%, transparent);
}
.about-html :deep(h1),
.about-html :deep(h2),
.about-html :deep(h3) {
  margin: 14px 0 6px;
  color: var(--text);
  font-size: 15px;
}
.about-html :deep(p),
.about-html :deep(ul),
.about-html :deep(ol) {
  margin: 6px 0;
}
.about-html :deep(img) {
  height: auto;
  border-radius: 8px;
}
.about-html :deep(table) {
  display: block;
  max-width: 100%;
  overflow-x: auto;
  border-collapse: collapse;
}
.about-html :deep(td),
.about-html :deep(th) {
  padding: 4px 8px;
  border: 1px solid var(--line);
}
.about .more {
  margin-top: 6px;
  color: var(--link);
  font-size: 13px;
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
.stats .mine {
  color: var(--text-3);
}
.stats .mine:hover {
  color: var(--link);
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
