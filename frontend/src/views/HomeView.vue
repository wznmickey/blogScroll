<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api, categoryName, meta, tagLabel, tagsInfo, type PostQuery } from '../api'
import { addChannel, anyHidden, channel, homeChannel, isMode, isShown, prefs, upgradeOldQuery, type ChannelId } from '../channels'
import ChannelBar from '../components/ChannelBar.vue'
import { useDragScroll } from '../composables/dragScroll'
import Waterfall from '../components/Waterfall.vue'
import { follows } from '../follow'
import { relativeTime } from '../format'
import { entries, passedOverIds, recentIds, recordShown } from '../library'
import { compareMarks, lastPosition, notePosition, type Mark, type Position } from '../positions'
import type { PostCard } from '../types'

const route = useRoute()
const router = useRouter()

const newSeed = () => Math.floor(Math.random() * 2 ** 31)
const seed = ref(newSeed())

interface FeedState {
  /** See channels.ts; '' = the first channel of the bar. */
  c: ChannelId
  /** Category, source and tag channels: mixed (default) or newest first. */
  latest: boolean
  /** Directory tag (开往: tech, life, ...), reached from a blog's page. */
  dirtag: string
  /** Newest-first feeds: resumed from this post (`sort_at.id`), '' = from the top. */
  from: string
}

// This view stays alive (<KeepAlive>) while other pages are open, so it only
// follows the URL while it is the current route.
const state = ref<FeedState>({ c: '', latest: false, dirtag: '', from: '' })
watch(
  () => [route.name, route.query],
  () => {
    if (route.name !== 'home') return
    const q = route.query
    // Searching has its own page now; older links used ?q= here.
    if (typeof q.q === 'string' && q.q.trim()) {
      router.replace({ path: '/search', query: { q: q.q } })
      return
    }
    const upgraded = upgradeOldQuery(q)
    if (upgraded) {
      router.replace({ path: '/', query: upgraded })
      return
    }
    const str = (v: unknown) => (typeof v === 'string' ? v.trim() : '')
    const next: FeedState = { c: str(q.c), latest: q.sort === 'latest', dirtag: str(q.dirtag), from: str(q.from) }
    // Ignore changes that only touch ?p= (the open post).
    if (JSON.stringify(next) !== JSON.stringify(state.value)) state.value = next
  },
  { immediate: true, deep: true },
)

const active = computed(() => state.value.c || homeChannel(tagsInfo.value))
const current = computed(() => channel(active.value, tagsInfo.value))
const filterValue = computed(() => active.value.slice(active.value.indexOf(':') + 1))
const followIds = computed(() => [...follows.value])
const isFollowing = computed(() => active.value === 'following')
const personal = computed(() => active.value === 'foryou' || active.value === 'explore')
const mixed = computed(() => (isMode(active.value) ? personal.value || active.value === 'discover' : !state.value.latest))

// Sub-categories of the open category, as a second row.
const subRow = computed(() => {
  if (current.value.type !== 'category') return null
  const top = filterValue.value.split('/')[0]
  const node = tagsInfo.value?.categories.find((c) => c.id === top)
  const children = (node?.children ?? []).filter((c) => c.posts > 0)
  return node && children.length ? { node, children } : null
})

// Personalisation is read when a feed (re)starts, not on every like.
const resetKey = computed(() =>
  [active.value, state.value.latest, state.value.dirtag, state.value.from, JSON.stringify(prefs.value.hide), seed.value].join('|') +
  (isFollowing.value ? `|${followIds.value.join('.')}` : ''),
)

// 上次看到哪里: newest-first feeds remember the newest and oldest post shown.
// Compared with how things stood when the feed was first opened this visit,
// newer posts are marked new, and reading can resume from the oldest.
const positionKey = computed(() => (mixed.value ? null : `${active.value}|${state.value.dirtag}`))
const baselines = new Map<string, Position | undefined>()
const baseline = ref<Position>()
const resumeMark = computed<Mark | null>(() => {
  const [sort_at, id] = state.value.from.split('.').map(Number)
  return Number.isFinite(sort_at) && Number.isInteger(id) ? { sort_at, id } : null
})
const newCount = ref(0)
const pastNew = ref(false)
const reachedOldest = ref(false)

function isFresh(p: PostCard) {
  return !resumeMark.value && !!baseline.value && compareMarks(p, baseline.value.newest) > 0
}

function startFeed() {
  const key = positionKey.value
  if (key && !baselines.has(key)) baselines.set(key, lastPosition(key))
  baseline.value = key ? baselines.get(key) : undefined
  newCount.value = 0
  pastNew.value = false
  reachedOldest.value = false
}

function onImpression(post: PostCard) {
  recordShown(post, current.value.type === 'category' ? categoryName(filterValue.value) : current.value.label)
  if (positionKey.value) notePosition(positionKey.value, post)
  if (baseline.value && compareMarks(post, baseline.value.oldest) <= 0) reachedOldest.value = true
}

const load = async (cursor: string | null) => {
  if (cursor === null) startFeed()
  const q: PostQuery = {
    cursor,
    startAt: resumeMark.value ?? undefined,
    hide: anyHidden(prefs.value.hide) ? { ...prefs.value.hide } : undefined,
    dirtag: state.value.dirtag || undefined,
    seed: seed.value,
    mode: active.value === 'explore' ? 'explore' : mixed.value ? 'discover' : 'latest',
  }
  switch (current.value.type) {
    case 'category':
      q.category = filterValue.value
      break
    case 'kind':
      q.kind = filterValue.value
      break
    case 'tag':
      q.tag = filterValue.value
      break
  }
  if (isFollowing.value) q.blogs = followIds.value
  if (personal.value) {
    q.follow = followIds.value
    q.like = recentIds('likes', 100)
    q.save = recentIds('saves', 100)
    q.read = recentIds('history', 50)
    q.shown = passedOverIds(200)
  }
  const page = await api.posts(q)
  if (baseline.value && !resumeMark.value) {
    const fresh = page.items.filter(isFresh).length
    newCount.value += fresh
    if (fresh < page.items.length || !page.next_cursor) pastNew.value = true
  }
  return page
}

function go(patch: Partial<FeedState>) {
  const next = { ...state.value, ...patch }
  const query: Record<string, string> = {}
  if (next.c && next.c !== homeChannel(tagsInfo.value)) query.c = next.c
  if (next.latest && !isMode(next.c || homeChannel(tagsInfo.value))) query.sort = 'latest'
  if (next.dirtag) query.dirtag = next.dirtag
  if (next.from) query.from = next.from
  router.push({ path: '/', query })
  window.scrollTo({ top: 0 })
}

function select(id: ChannelId) {
  if (id === active.value) {
    // Tapping the open channel again reshuffles, like pulling to refresh.
    if (mixed.value) seed.value = newSeed()
    window.scrollTo({ top: 0, behavior: 'smooth' })
    return
  }
  go({ c: id, latest: false, from: '' })
}

function resume() {
  const o = baseline.value?.oldest
  if (o) go({ from: `${o.sort_at}.${o.id}` })
}

const subsEl = ref<HTMLElement>()
useDragScroll(subsEl)

const canAdd = computed(() => !isShown(active.value, tagsInfo.value))
const hasSignals = computed(
  () =>
    followIds.value.length > 0 ||
    (['likes', 'saves', 'history'] as const).some((n) => entries(n).value.length > 0),
)
const title = computed(() =>
  current.value.type === 'category' ? categoryName(filterValue.value) : current.value.title,
)
</script>

<template>
  <div class="container">
    <div class="feed-nav">
      <ChannelBar :active="active" @select="select" />
      <div v-if="subRow || !isMode(active) || state.dirtag" class="sub">
        <div v-if="subRow" ref="subsEl" class="subs" role="toolbar" :aria-label="`${subRow.node.label}的子分类`">
          <button
            class="sub-chip"
            :class="{ on: filterValue === subRow.node.id }"
            @click="go({ c: `cat:${subRow.node.id}`, from: '' })"
          >
            全部{{ subRow.node.label }}
          </button>
          <button
            v-for="c in subRow.children"
            :key="c.id"
            class="sub-chip"
            :class="{ on: filterValue === c.id }"
            @click="go({ c: `cat:${c.id}`, from: '' })"
          >
            {{ c.label }}
          </button>
        </div>
        <span v-else-if="!isMode(active)" class="context">{{ title }}</span>
        <span v-if="state.dirtag" class="context">
          开往标签：{{ tagLabel(state.dirtag) }}
          <button type="button" class="link" @click="go({ dirtag: '' })">清除</button>
        </span>
        <span class="spacer" />
        <button v-if="canAdd" class="link" @click="addChannel(active, tagsInfo)">+ 添加到频道栏</button>
        <div v-if="!isMode(active)" class="sort" role="group" aria-label="排序">
          <button :class="{ on: !state.latest }" :aria-pressed="!state.latest" @click="go({ latest: false, from: '' })">
            综合
          </button>
          <button :class="{ on: state.latest }" :aria-pressed="state.latest" @click="go({ latest: true, from: '' })">
            最新
          </button>
        </div>
      </div>
    </div>

    <div v-if="resumeMark" class="position-bar">
      <span>从上次看到的地方继续</span>
      <button class="link" @click="go({ from: '' })">回到最新 ↑</button>
    </div>
    <div v-else-if="baseline && (newCount > 0 || !reachedOldest)" class="position-bar">
      <span v-if="newCount > 0">上次之后有 {{ newCount }}{{ pastNew ? '' : '+' }} 篇新文章</span>
      <span v-else>上次之后没有新文章</span>
      <button v-if="!reachedOldest" class="link resume" @click="resume">
        从上次看到的地方继续：《{{ baseline.oldest.title }}》 · {{ relativeTime(baseline.at / 1000) }}看到 ↓
      </button>
    </div>
    <p v-if="active === 'explore'" class="explore-note">
      <template v-if="hasSignals">避开你读过、喜欢、收藏和关注的方向，看看平时不会遇到的博客和分类。</template>
      <template v-else>反推荐会避开你读过、喜欢、收藏和关注的方向；你还没有这些记录，所以现在和「发现」差不多。</template>
    </p>
    <div v-if="isFollowing && !followIds.length" class="empty follow-empty">
      <p>还没有关注任何博主。</p>
      <p class="sub-note">在文章或博客页点「关注」，他们的新文章会出现在这里；关注记录只保存在这台设备的浏览器里。</p>
      <RouterLink to="/blogs" class="btn btn-ghost">去发现博主</RouterLink>
    </div>
    <Waterfall v-else :load="load" :reset-key="resetKey" :is-fresh="isFresh" @impression="onImpression">
      <template #empty>
        <div class="empty">
          <p v-if="!isMode(active) || state.dirtag || anyHidden(prefs.hide)">没有符合条件的文章</p>
          <p v-else-if="isFollowing">关注的博主最近没有新文章</p>
          <template v-else>
            <p>还没有文章。</p>
            <p class="sub-note">
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
  margin: 0 -24px 12px;
  padding: 8px 24px 10px;
  background: color-mix(in srgb, var(--bg-page) 92%, transparent);
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
}
.sub {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 8px;
  min-height: 28px;
  font-size: 13px;
}
.subs {
  display: flex;
  gap: 6px;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
}
.subs::-webkit-scrollbar {
  display: none;
}
.subs {
  user-select: none;
}
.sub-chip {
  flex: none;
  height: 26px;
  padding: 0 12px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
  white-space: nowrap;
}
.sub-chip.on {
  background: var(--text);
  color: var(--bg);
  font-weight: 600;
}
.context {
  color: var(--text-2);
  white-space: nowrap;
}
.spacer {
  flex: 1;
}
.link {
  color: var(--link);
  font-size: 13px;
  white-space: nowrap;
}
.context .link {
  margin-left: 6px;
}
.sort {
  display: flex;
  flex: none;
  padding: 2px;
  border-radius: 999px;
  background: var(--surface);
}
.sort button {
  height: 24px;
  padding: 0 10px;
  border-radius: 999px;
  color: var(--text-2);
  font-size: 12px;
}
.sort button.on {
  background: var(--bg);
  color: var(--text);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
}
.position-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 4px 14px;
  margin: -4px 4px 12px;
  color: var(--text-2);
  font-size: 13px;
}
.position-bar .resume {
  overflow: hidden;
  max-width: 100%;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.explore-note {
  margin: -4px 4px 12px;
  color: var(--text-3);
  font-size: 13px;
}
.follow-empty {
  padding: 48px 0;
}
.follow-empty .btn {
  margin-top: 12px;
}
.empty {
  text-align: center;
  color: var(--text-2);
}
.empty p {
  margin: 4px 0;
}
.sub-note {
  color: var(--text-3);
  font-size: 13px;
}
@media (max-width: 639px) {
  .feed-nav {
    margin: 0 -6px 6px;
    padding: 2px 8px 6px;
  }
  .sub {
    margin-top: 4px;
    gap: 8px;
  }
}
</style>
