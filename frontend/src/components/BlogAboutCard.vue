<script setup lang="ts">
// A blogger's profile card: next to one of their posts (where links stay on
// this site and the only way out is the post's own "阅读原文"), and for the
// site's maintainer on the about page, which adds its own links below.

import { computed, ref, watch } from 'vue'
import { blogDetail } from '../api'
import { relativeTime } from '../format'
import { socialLabels } from '../social'
import type { BlogDetail, BlogRef } from '../types'
import BlogAvatar from './BlogAvatar.vue'

const props = defineProps<{ blog: BlogRef }>()

const detail = ref<BlogDetail | null>(null)
const socialNames = computed(() => socialLabels(detail.value?.social ?? []))
const expanded = ref(false)
watch(
  () => props.blog.id,
  async (id) => {
    detail.value = null
    expanded.value = false
    try {
      detail.value = await blogDetail(id)
    } catch {
      // The card still shows the name and avatar from the post itself.
    }
  },
  { immediate: true },
)

const kind = computed(() => (detail.value?.kind && detail.value.kind.id !== 'blog' ? detail.value.kind.label : null))
const about = computed(() => detail.value?.about_text ?? null)
const longAbout = computed(() => (about.value?.length ?? 0) > 110)
</script>

<template>
  <section class="about-card" aria-label="关于博主">
    <header class="head">
      <RouterLink :to="`/blog/${blog.id}`" class="who">
        <BlogAvatar :name="blog.name" :icon="detail?.icon ?? blog.icon" :size="44" />
        <span class="names">
          <span class="name">{{ blog.name }}</span>
          <span v-if="detail" class="meta">
            <template v-if="kind">{{ kind }} · </template>{{ detail.post_count }} 篇
            <template v-if="detail.last_post_at"> · {{ relativeTime(detail.last_post_at) }}更新</template>
          </span>
        </span>
      </RouterLink>
    </header>

    <p v-if="detail?.description" class="desc">{{ detail.description }}</p>

    <div v-if="about" class="about">
      <p :class="{ clamped: longAbout && !expanded }">{{ about }}</p>
      <button v-if="longAbout" type="button" class="more" @click="expanded = !expanded">
        {{ expanded ? '收起' : '展开' }}
      </button>
    </div>

    <p v-if="detail?.categories?.length" class="chips">
      <span class="label">常写</span>
      <RouterLink
        v-for="c in detail.categories"
        :key="c.id"
        :to="{ path: '/', query: { c: `cat:${c.id}` } }"
        class="chip topic"
        :title="c.share ? `约 ${Math.round(c.share * 100)}% 的文章` : undefined"
      >
        {{ c.label }}
      </RouterLink>
    </p>
    <p v-if="detail?.top_tags?.length" class="chips tags">
      <span class="label">标签</span>
      <RouterLink
        v-for="t in detail.top_tags.slice(0, 6)"
        :key="t"
        :to="{ path: '/', query: { c: `tag:${t}` } }"
        class="tag"
        >#{{ t }}</RouterLink
      >
    </p>

    <p v-if="detail?.social.length" class="social">
      <a v-for="(s, i) in detail.social" :key="s.url" :href="s.url" target="_blank" rel="noopener me">{{
        socialNames[i]
      }}</a>
    </p>

    <footer v-if="$slots.default" class="extra"><slot /></footer>
  </section>
</template>

<style scoped>
.about-card {
  margin-top: 20px;
  padding: 14px 16px;
  border-radius: 14px;
  background: var(--surface);
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.who {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.names {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}
.meta {
  color: var(--text-3);
  font-size: 12px;
}
.desc {
  margin: 10px 0 0;
  color: var(--text-2);
  font-size: 13px;
  line-height: 1.6;
}
.about {
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--line);
}
.about p {
  margin: 0;
  color: var(--text-2);
  font-size: 13px;
  line-height: 1.7;
  white-space: pre-line;
}
.about p.clamped {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 4;
  line-clamp: 4;
  overflow: hidden;
}
.more {
  margin-top: 2px;
  color: var(--link);
  font-size: 12px;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 6px;
  margin: 10px 0 0;
}
.chip {
  padding: 1px 9px;
  border-radius: 999px;
  background: var(--bg);
  color: var(--text-2);
  font-size: 12px;
}
.chip.topic {
  background: var(--accent-soft);
  color: var(--accent);
}
.chip:hover {
  color: var(--text);
}
.label {
  color: var(--text-3);
  font-size: 12px;
}
.chips.tags {
  gap: 4px 10px;
  margin-top: 6px;
}
.tag {
  color: var(--link);
  font-size: 12px;
}
.social {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  margin: 10px 0 0;
  font-size: 12px;
}
.social a {
  color: var(--link);
}
.extra {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 14px;
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--line);
  font-size: 13px;
}
</style>
