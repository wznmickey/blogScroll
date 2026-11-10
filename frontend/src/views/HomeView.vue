<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api, meta, tagLabel, tagsInfo, topicLabel } from '../api'
import BlogAvatar from '../components/BlogAvatar.vue'
import ChipBar, { type Chip } from '../components/ChipBar.vue'
import FollowButton from '../components/FollowButton.vue'
import Waterfall from '../components/Waterfall.vue'
import { follows, recentlyRead } from '../follow'
import { hostOf } from '../format'
import type { BlogSummary, FeedMode } from '../types'

const route = useRoute()
const router = useRouter()

const newSeed = () => Math.floor(Math.random() * 2 ** 31)
const seed = ref(newSeed())

interface FeedState {
  tab: FeedMode
  /** Content area channel (dev, travel, ...); '' = all. */
  topic: string
  /** Source kind (blog, news, podcast, ...); '' = all. */
  kind: string
  /** Directory tag (开往: tech, life, ...), reached from a blog's page. */
  tag: string
  cover: boolean
  q: string
  /** A post tag, reached by clicking #tag on a post. */
  category: string
}

// This view stays alive (<KeepAlive>) while other pages are open, so it only
// follows the URL while it is the current route.
const state = ref<FeedState>({ tab: 'discover', topic: '', kind: '', tag: '', cover: false, q: '', category: '' })
watch(
  () => [route.name, route.query],
  () => {
    if (route.name !== 'home') return
    const str = (v: unknown) => (typeof v === 'string' ? v.trim() : '')
    const tab = str(route.query.tab)
    const next: FeedState = {
      tab: tab === 'latest' || tab === 'following' ? tab : 'discover',
      topic: str(route.query.topic),
      kind: str(route.query.kind),
      tag: str(route.query.tag),
      cover: route.query.cover === '1',
      q: str(route.query.q),
      category: str(route.query.category),
    }
    // Ignore changes that only touch ?p= (the open post).
    if (JSON.stringify(next) !== JSON.stringify(state.value)) state.value = next
  },
  { immediate: true, deep: true },
)

const s = computed(() => state.value)
const filtering = computed(() => !!(s.value.q || s.value.category))
const following = computed(() => s.value.tab === 'following' && !filtering.value)
const followIds = computed(() => [...follows.value])
// Personalisation is read when a feed (re)starts, not on every follow click.
const resetKey = computed(() =>
  filtering.value
    ? `q:${s.value.q}|c:${s.value.category}`
    : [s.value.tab, s.value.topic, s.value.kind, s.value.tag, s.value.cover, seed.value].join('|') +
      (following.value ? `|${followIds.value.join('.')}` : ''),
)

const load = (cursor: string | null) => {
  if (filtering.value) {
    return api.posts({ mode: 'latest', cursor, q: s.value.q || undefined, category: s.value.category || undefined })
  }
  const common = {
    cursor,
    topic: s.value.topic || undefined,
    kind: s.value.kind || undefined,
    tag: s.value.tag || undefined,
    cover: s.value.cover,
  }
  if (following.value) return api.posts({ ...common, mode: 'following', blogs: followIds.value })
  return api.posts({
    ...common,
    mode: s.value.tab,
    seed: seed.value,
    follow: s.value.tab === 'discover' ? followIds.value : undefined,
    read: s.value.tab === 'discover' ? recentlyRead() : undefined,
  })
}

const tabs: { id: FeedMode; label: string }[] = [
  { id: 'discover', label: '推荐' },
  { id: 'latest', label: '最新' },
  { id: 'following', label: '关注' },
]

const chips = computed<Chip[]>(() => [
  { value: '', label: '全部' },
  ...(tagsInfo.value?.topics ?? []).filter((t) => t.posts > 0).map((t) => ({ value: t.id, label: t.label })),
])
const kinds = computed(() => tagsInfo.value?.kinds ?? [])

function go(patch: Partial<Pick<FeedState, 'tab' | 'topic' | 'kind' | 'tag' | 'cover'>>) {
  const next = { ...s.value, ...patch }
  const query: Record<string, string> = {}
  if (next.tab !== 'discover') query.tab = next.tab
  if (next.topic) query.topic = next.topic
  if (next.kind) query.kind = next.kind
  if (next.tag) query.tag = next.tag
  if (next.cover) query.cover = '1'
  router.push({ path: '/', query })
  window.scrollTo({ top: 0 })
}

function selectTab(id: FeedMode) {
  if (id === s.value.tab) {
    // Tapping the active tab again reshuffles, like pulling to refresh.
    if (id === 'discover') seed.value = newSeed()
    window.scrollTo({ top: 0, behavior: 'smooth' })
    return
  }
  go({ tab: id })
}

// Searching also looks for bloggers: name, address, description, about text.
const blogHits = ref<BlogSummary[]>([])
const moreBlogs = ref(false)
watch(
  () => s.value.q,
  async (q) => {
    blogHits.value = []
    moreBlogs.value = false
    if (!q) return
    try {
      const page = await api.blogs({ q, limit: 8 })
      if (q !== s.value.q) return
      blogHits.value = page.items
      moreBlogs.value = !!page.next_cursor
    } catch {
      // Post results still show.
    }
  },
  { immediate: true },
)
</script>

<template>
  <div class="container">
    <template v-if="filtering">
      <div class="filter-head">
        <span v-if="s.category">#{{ s.category }}</span>
        <span v-else>“{{ s.q }}” 的搜索结果</span>
        <RouterLink to="/" class="clear">清除</RouterLink>
      </div>
      <section v-if="blogHits.length" class="blog-hits" aria-label="博主">
        <h2>
          博主
          <RouterLink v-if="moreBlogs" :to="{ path: '/blogs', query: { q: s.q } }" class="all">查看全部</RouterLink>
        </h2>
        <ul>
          <li v-for="b in blogHits" :key="b.id">
            <RouterLink :to="`/blog/${b.id}`" class="hit">
              <BlogAvatar :name="b.name" :icon="b.icon" :size="40" />
              <span class="hit-text">
                <span class="hit-name">{{ b.name }}</span>
                <span class="hit-sub">{{ b.description || hostOf(b.homepage) }}</span>
              </span>
              <FollowButton :blog-id="b.id" small />
            </RouterLink>
          </li>
        </ul>
        <h2 class="posts-head">文章</h2>
      </section>
    </template>
    <div v-else class="feed-nav">
      <nav class="tabs" aria-label="排序">
        <button
          v-for="t in tabs"
          :key="t.id"
          class="tab"
          :class="{ active: s.tab === t.id }"
          :aria-current="s.tab === t.id ? 'page' : undefined"
          @click="selectTab(t.id)"
        >
          {{ t.label }}
        </button>
      </nav>
      <ChipBar :chips="chips" :active="s.topic" label="领域" class="channels" @select="(topic) => go({ topic })">
        <span class="divider" aria-hidden="true" />
        <select
          v-if="kinds.length > 1"
          class="kind"
          :class="{ on: s.kind }"
          :value="s.kind"
          aria-label="来源类型"
          @change="go({ kind: ($event.target as HTMLSelectElement).value })"
        >
          <option value="">全部来源</option>
          <option v-for="k in kinds" :key="k.id" :value="k.id">{{ k.label }}</option>
        </select>
        <button class="toggle" :class="{ on: s.cover }" :aria-pressed="s.cover" @click="go({ cover: !s.cover })">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="3" y="4" width="18" height="16" rx="3" />
            <circle cx="9" cy="10" r="2" />
            <path d="m21 16-5-5-9 9" />
          </svg>
          只看有图
        </button>
      </ChipBar>
    </div>
    <p v-if="!filtering && s.tag" class="tag-note">
      开往标签：{{ tagLabel(s.tag) }}
      <button type="button" class="clear" @click="go({ tag: '' })">清除</button>
    </p>

    <div v-if="following && !followIds.length" class="empty follow-empty">
      <p>还没有关注任何博主。</p>
      <p class="sub">在文章或博客页点「关注」，他们的新文章会出现在这里；关注记录只保存在这台设备的浏览器里。</p>
      <RouterLink to="/blogs" class="btn btn-ghost">去发现博主</RouterLink>
    </div>
    <Waterfall v-else :load="load" :reset-key="resetKey">
      <template #empty>
        <div class="empty">
          <p v-if="filtering || s.topic || s.kind || s.tag || s.cover">
            没有符合条件的文章<template v-if="s.topic">（{{ topicLabel(s.topic) }}）</template>
          </p>
          <p v-else-if="following">关注的博主最近没有新文章</p>
          <template v-else>
            <p>还没有文章。</p>
            <p class="sub">
              后台正在慢慢抓取 RSS（已收录 {{ meta?.stats.blogs ?? 0 }} 个博客），稍后刷新看看。
            </p>
          </template>
        </div>
      </template>
    </Waterfall>
  </div>
</template>

<style scoped>
.feed-nav {
  position: sticky;
  top: var(--header-h);
  z-index: 10;
  display: flex;
  align-items: center;
  gap: 16px;
  margin: 0 -24px 12px;
  padding: 8px 24px 12px;
  background: color-mix(in srgb, var(--bg-page) 92%, transparent);
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
}
.tabs {
  display: flex;
  flex: none;
  gap: 4px;
}
.tab {
  height: 34px;
  padding: 0 16px;
  border-radius: 999px;
  color: var(--text-2);
  font-size: 15px;
}
.tab:hover {
  color: var(--text);
}
.tab.active {
  background: var(--surface);
  color: var(--text);
  font-weight: 600;
}
.channels {
  flex: 1;
  min-width: 0;
}
.divider {
  flex: none;
  width: 1px;
  height: 16px;
  background: var(--line);
}
.toggle {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 30px;
  padding: 0 12px;
  border-radius: 999px;
  border: 1px solid var(--line);
  color: var(--text-2);
  font-size: 13px;
  white-space: nowrap;
}
.kind {
  flex: none;
  height: 30px;
  padding: 0 10px;
  border-radius: 999px;
  border: 1px solid var(--line);
  background: var(--bg);
  color: var(--text-2);
  font: inherit;
  font-size: 13px;
}
.kind.on {
  border-color: var(--accent);
  color: var(--accent);
}
.tag-note {
  margin: -4px 4px 12px;
  color: var(--text-2);
  font-size: 13px;
}
.tag-note .clear {
  margin-left: 8px;
}
.blog-hits h2 {
  display: flex;
  align-items: baseline;
  gap: 12px;
  margin: 0 4px 10px;
  font-size: 15px;
}
.blog-hits .all {
  color: var(--link);
  font-size: 13px;
  font-weight: 400;
}
.blog-hits ul {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 10px;
  margin: 0 0 20px;
  padding: 0;
  list-style: none;
}
.hit {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: var(--card);
}
.hit:hover {
  border-color: color-mix(in srgb, var(--text) 25%, transparent);
}
.hit-text {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}
.hit-name,
.hit-sub {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hit-name {
  font-weight: 600;
  font-size: 14px;
}
.hit-sub {
  color: var(--text-3);
  font-size: 12px;
}
.posts-head {
  margin-top: 4px !important;
}
.follow-empty {
  padding: 48px 0;
}
.follow-empty .btn {
  margin-top: 12px;
}
.toggle.on {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}
.filter-head {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 12px 4px 16px;
  font-size: 16px;
  font-weight: 600;
}
.clear {
  color: var(--link);
  font-size: 14px;
  font-weight: 400;
}
button.clear {
  font-size: 13px;
}
.empty {
  text-align: center;
  color: var(--text-2);
}
.empty p {
  margin: 4px 0;
}
.empty .sub {
  color: var(--text-3);
  font-size: 13px;
}
@media (max-width: 639px) {
  .feed-nav {
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
    margin: 0 -6px 6px;
    padding: 2px 8px 8px;
  }
  .tabs {
    justify-content: center;
  }
  .tab {
    height: 30px;
  }
  .tab.active {
    background: none;
    position: relative;
  }
  .tab.active::after {
    content: '';
    position: absolute;
    left: 50%;
    bottom: 0;
    width: 18px;
    height: 3px;
    border-radius: 2px;
    background: var(--accent);
    transform: translateX(-50%);
  }
  .toggle {
    height: 28px;
  }
}
</style>
