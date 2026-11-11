<script setup lang="ts">
// 大家在看: what other readers took to lately (opened, read, liked, saved,
// followed, searched; backend/src/stats.rs): bloggers and posts, others' own
// feeds that the site does not list (anyone may add them), searches, tags,
// keywords, categories. From GET /api/trending, or trending.json on the
// static site (made at each publish).
import { computed, onMounted, ref } from 'vue'
import { api, categoryName } from '../api'
import BlogAvatar from '../components/BlogAvatar.vue'
import FollowButton from '../components/FollowButton.vue'
import Waterfall from '../components/Waterfall.vue'
import { blogPath, hostOf } from '../format'
import { recordShown } from '../library'
import { options } from '../options'
import { addOwn, ownSources } from '../sources'
import type { PostCard, Trending } from '../types'

document.title = '大家在看'

const data = ref<Trending | null>(null)
const failed = ref(false)
onMounted(async () => {
  try {
    data.value = await api.trending()
  } catch {
    failed.value = true
  }
})

const nothing = computed(
  () =>
    !!data.value &&
    !data.value.posts.length &&
    !data.value.blogs.length &&
    !data.value.reader_feeds.length &&
    !data.value.searches.length,
)

const load = async (cursor: string | null) => {
  const all: PostCard[] = data.value?.posts ?? []
  const offset = Number(cursor) || 0
  return { items: all.slice(offset, offset + 30), next_cursor: offset + 30 < all.length ? String(offset + 30) : null }
}

const mine = computed(() => new Set(ownSources.value.map((s) => s.url)))
function subscribe(f: Trending['reader_feeds'][number]) {
  addOwn({ source: { url: f.feed, kind: 'feed', title: f.blog.name, homepage: f.blog.homepage, at: Date.now() } })
}
const score = (n: number) => (n >= 100 ? Math.round(n) : Math.round(n * 10) / 10)
</script>

<template>
  <div class="container">
    <article class="trending">
      <header class="head">
        <h1>大家在看</h1>
        <p v-if="data" class="lead">
          最近 {{ data.days }} 天，其他读者打开、喜欢、收藏、关注和搜索最多的（只算开着统计的浏览器）。今天 {{ data.visitors.today }}
          位访客，这 7 天共 {{ data.visitors.week }} 人次。
        </p>
      </header>
      <p v-if="failed" class="empty">暂时还没有统计数据。</p>
      <p v-else-if="!data" class="empty"><span class="spinner" /></p>
      <p v-else-if="nothing" class="empty">还没有足够的数据：读者多看一会儿，这里就会有内容。</p>
      <template v-if="data && !nothing">
        <div class="terms">
          <section v-if="data.searches.length">
            <h2>大家在搜</h2>
            <p class="chips">
              <RouterLink v-for="t in data.searches" :key="t.term" :to="{ path: '/search', query: { q: t.term } }" class="chip">
                {{ t.term }}<small v-if="options.showHeat"> {{ t.n }}</small>
              </RouterLink>
            </p>
          </section>
          <section v-if="data.tags.length">
            <h2>热门标签</h2>
            <p class="chips">
              <RouterLink v-for="t in data.tags" :key="t.term" :to="{ path: '/', query: { c: `tag:${t.term}` } }" class="chip">
                #{{ t.term }}
              </RouterLink>
            </p>
          </section>
          <section v-if="data.keywords.length">
            <h2>热门关键词</h2>
            <p class="chips">
              <RouterLink v-for="t in data.keywords" :key="t.term" :to="{ path: '/search', query: { q: t.term } }" class="chip">
                {{ t.term }}
              </RouterLink>
            </p>
          </section>
          <section v-if="data.categories.length">
            <h2>热门分类</h2>
            <p class="chips">
              <RouterLink
                v-for="c in data.categories"
                :key="c.category.id"
                :to="{ path: '/', query: { c: `cat:${c.category.id}` } }"
                class="chip"
              >
                {{ categoryName(c.category) }}<small v-if="options.showHeat"> {{ score(c.n) }}</small>
              </RouterLink>
            </p>
          </section>
        </div>

        <section v-if="data.blogs.length">
          <h2>热门博主</h2>
          <ul class="blogs">
            <li v-for="b in data.blogs" :key="b.id">
              <RouterLink :to="blogPath(b)" class="who">
                <BlogAvatar :name="b.name" :icon="b.icon" :size="40" />
                <span class="who-text">
                  <span class="name">{{ b.name }}</span>
                  <small>
                    {{ b.followers ? `${b.followers} 人关注` : hostOf(b.homepage) }}
                    <template v-if="options.showHeat"> · 热度 {{ score(b.score) }}</template>
                  </small>
                </span>
              </RouterLink>
              <FollowButton :blog-id="b.id" :blog-key="b.key" small />
            </li>
          </ul>
        </section>

        <section v-if="data.reader_feeds.length">
          <h2>读者在订阅</h2>
          <p class="hint">本站没有收录、读者自己添加的博客。加到你的「我添加的来源」，它的文章就会出现在你的推荐、最新和订阅频道里。</p>
          <ul class="feeds">
            <li v-for="f in data.reader_feeds" :key="f.feed">
              <div class="feed-head">
                <a :href="f.blog.homepage" target="_blank" rel="noopener" class="who">
                  <BlogAvatar :name="f.blog.name" :icon="f.blog.icon" :size="36" />
                  <span class="who-text">
                    <span class="name">{{ f.blog.name }}</span>
                    <small>{{ f.subscribers }} 位读者订阅 · {{ hostOf(f.blog.homepage) }}</small>
                  </span>
                </a>
                <button class="btn btn-ghost add" :disabled="mine.has(f.feed)" @click="subscribe(f)">
                  {{ mine.has(f.feed) ? '已添加' : '加到我的来源' }}
                </button>
              </div>
              <ul class="titles">
                <li v-for="p in f.posts" :key="p.id">
                  <a :href="p.url" target="_blank" rel="noopener">{{ p.title }}</a>
                </li>
              </ul>
            </li>
          </ul>
        </section>

        <section v-if="data.posts.length" class="posts">
          <h2>热门文章</h2>
          <Waterfall :load="load" reset-key="trending" @impression="(p) => recordShown(p, '大家在看')" />
        </section>
      </template>
    </article>
  </div>
</template>

<style scoped>
.trending {
  margin: 24px auto 64px;
}
.head,
.terms,
section:not(.posts) {
  max-width: 960px;
  margin-right: auto;
  margin-left: auto;
}
h1 {
  margin: 0 0 6px;
  font-size: 26px;
}
h2 {
  margin: 24px 0 8px;
  font-size: 17px;
}
.lead,
.hint {
  margin: 4px 0;
  color: var(--text-3);
  font-size: 13px;
}
.empty {
  margin: 48px 0;
  color: var(--text-3);
  text-align: center;
}
.terms {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: 0 24px;
}
.terms section {
  max-width: none;
  margin: 0;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 0;
}
.chip {
  padding: 3px 10px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
}
.chip:hover {
  color: var(--accent);
}
.chip small {
  margin-left: 3px;
  color: #f97316;
}
.blogs {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: 10px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.blogs li,
.feeds > li {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 12px;
  background: var(--surface);
}
.who {
  display: flex;
  flex: 1;
  align-items: center;
  gap: 10px;
  min-width: 0;
  color: var(--text);
}
.who-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.name {
  overflow: hidden;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.who small {
  color: var(--text-3);
  font-size: 12px;
}
.feeds {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
}
.feeds > li {
  flex-direction: column;
  align-items: stretch;
}
.feed-head {
  display: flex;
  align-items: center;
  gap: 10px;
}
.add {
  flex: none;
}
.titles {
  margin: 0;
  padding-left: 46px;
  color: var(--text-2);
  font-size: 13px;
  line-height: 1.7;
}
.titles a {
  color: inherit;
}
.titles a:hover {
  color: var(--link);
}
</style>
