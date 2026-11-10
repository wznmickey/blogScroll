<script setup lang="ts">
// Saves, likes, history, 刷到过 (shown on screen, opened or not) and followed
// blogs, all kept in this browser, plus backup and restore of them.
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api'
import { downloadBackup, parseBackup, restoreBackup, type Backup } from '../backup'
import BlogAvatar from '../components/BlogAvatar.vue'
import ChipBar, { type Chip } from '../components/ChipBar.vue'
import FollowButton from '../components/FollowButton.vue'
import Waterfall from '../components/Waterfall.vue'
import { follows } from '../follow'
import { hostOf } from '../format'
import { clear, entries, LIST_LABELS, searchList, setSettings, settings, type ListName } from '../library'
import type { BlogSummary, Page, PostCard } from '../types'

type Tab = ListName | 'follows' | 'backup'

const route = useRoute()
const router = useRouter()
const TABS: Tab[] = ['saves', 'likes', 'history', 'seen', 'follows', 'backup']
const tab = computed<Tab>(() => (TABS.includes(route.query.tab as Tab) ? (route.query.tab as Tab) : 'saves'))
const isList = computed(() => tab.value in LIST_LABELS)

const lists = {
  saves: entries('saves'),
  likes: entries('likes'),
  history: entries('history'),
  seen: entries('seen'),
}
const chips = computed<Chip[]>(() => [
  { value: 'saves', label: '收藏', count: lists.saves.value.length },
  { value: 'likes', label: '喜欢', count: lists.likes.value.length },
  { value: 'history', label: '历史', count: lists.history.value.length },
  { value: 'seen', label: '刷到过', count: lists.seen.value.length },
  { value: 'follows', label: '关注', count: follows.value.length },
  { value: 'backup', label: '备份与恢复' },
])

function setTab(t: string) {
  router.replace({ query: t === 'saves' ? {} : { tab: t } })
}

// Lists are read when a tab opens, so unliking a post does not shift the
// cards under the cursor; clearing or restoring reloads.
const version = ref(0)
const filter = ref('')
const filterKey = ref('')
let filterTimer: ReturnType<typeof setTimeout> | undefined
watch(filter, (v) => {
  clearTimeout(filterTimer)
  filterTimer = setTimeout(() => (filterKey.value = v.trim()), 250)
})
watch(tab, () => (filter.value = filterKey.value = ''))
const loadList = (cursor: string | null): Promise<Page<PostCard>> => {
  const all = searchList(tab.value as ListName, filterKey.value).map((e) => e.post)
  const offset = Number(cursor) || 0
  return Promise.resolve({
    items: all.slice(offset, offset + 30),
    next_cursor: offset + 30 < all.length ? String(offset + 30) : null,
  })
}

function clearList(name: ListName) {
  if (!confirm(`清空全部${LIST_LABELS[name]}？此操作不能撤销，可以先备份。`)) return
  clear(name)
  version.value++
}

const historyOn = computed({
  get: () => settings.value.history,
  set: (v: boolean) => setSettings({ history: v }),
})
const seenOn = computed({
  get: () => settings.value.seen,
  set: (v: boolean) => setSettings({ seen: v }),
})

// Followed blogs.
const followed = ref<BlogSummary[]>([])
const loadingFollows = ref(false)
watch(
  () => tab.value === 'follows',
  async (open) => {
    if (!open) return
    const ids = [...follows.value]
    followed.value = []
    if (!ids.length) return
    loadingFollows.value = true
    try {
      followed.value = (await api.blogs({ ids, limit: 200 })).items
    } catch {
      followed.value = []
    } finally {
      loadingFollows.value = false
    }
  },
  { immediate: true },
)

// Backup and restore.
const pending = ref<Backup | null>(null)
const restoreError = ref('')
const restored = ref('')
const fileInput = ref<HTMLInputElement>()

async function pickFile(e: Event) {
  const file = (e.target as HTMLInputElement).files?.[0]
  restoreError.value = ''
  restored.value = ''
  pending.value = null
  if (!file) return
  try {
    pending.value = parseBackup(await file.text())
  } catch (err) {
    restoreError.value = err instanceof Error ? err.message : String(err)
  } finally {
    if (fileInput.value) fileInput.value.value = ''
  }
}

function applyRestore(mode: 'merge' | 'replace') {
  if (!pending.value) return
  if (mode === 'replace' && !confirm('覆盖会用备份替换这台设备上现有的关注、收藏、喜欢、历史、刷到过和频道设置，确定吗？'))
    return
  restoreBackup(pending.value, mode)
  restored.value = mode === 'merge' ? '已合并到现有数据' : '已用备份覆盖现有数据'
  pending.value = null
  version.value++
}

const exportedAt = (b: Backup) => (b.exported_at ? new Date(b.exported_at).toLocaleString('zh-CN') : '未知时间')
</script>

<template>
  <div class="container">
    <header class="head">
      <h1>我的</h1>
      <p class="note">这些记录只保存在这台设备的浏览器里，不需要账号；换设备或清理浏览器前，可以先备份。</p>
      <ChipBar :chips="chips" :active="tab" label="分类" @select="setTab" />
    </header>

    <template v-if="isList">
      <div class="bar">
        <label v-if="tab === 'history'" class="switch">
          <input v-model="historyOn" type="checkbox" />
          记录浏览历史
        </label>
        <label v-if="tab === 'seen'" class="switch" title="卡片在屏幕上停留片刻就算刷到过，点没点开都记">
          <input v-model="seenOn" type="checkbox" />
          记录刷到过的文章
        </label>
        <input
          v-model="filter"
          type="search"
          class="filter"
          :placeholder="`在${LIST_LABELS[tab as ListName]}中搜索`"
          :aria-label="`在${LIST_LABELS[tab as ListName]}中搜索`"
        />
        <span class="spacer" />
        <button v-if="lists[tab as ListName].value.length" class="link danger" @click="clearList(tab as ListName)">
          清空
        </button>
      </div>
      <Waterfall :load="loadList" :reset-key="`${tab}|${version}|${filterKey}`">
        <template #empty>
          <div class="empty">
            <p v-if="filterKey">没有匹配“{{ filterKey }}”的文章</p>
            <p v-else-if="tab === 'saves'">还没有收藏。打开文章，点下方的 ☆ 收藏。</p>
            <p v-else-if="tab === 'likes'">还没有喜欢的文章。点卡片或文章里的 ♡ 试试，「推荐」会更懂你。</p>
            <template v-else-if="tab === 'seen'">
              <p v-if="seenOn">还没有刷到过的文章。在首页、博客页和搜索结果里停留过的卡片都会记在这里，点没点开都算。</p>
              <p v-else>刷到过的记录已关闭。</p>
            </template>
            <p v-else-if="historyOn">还没有浏览记录。</p>
            <p v-else>浏览历史已关闭。</p>
          </div>
        </template>
      </Waterfall>
    </template>

    <template v-else-if="tab === 'follows'">
      <p v-if="!follows.length" class="empty">
        还没有关注任何博主。<RouterLink to="/blogs" class="link">去发现博主</RouterLink>
      </p>
      <p v-else-if="loadingFollows" class="empty"><span class="spinner" /></p>
      <ul v-else class="blog-list">
        <li v-for="b in followed" :key="b.id">
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
    </template>

    <section v-else class="backup">
      <div class="card">
        <h2>备份</h2>
        <p>
          把关注（{{ follows.length }}）、收藏（{{ lists.saves.value.length }}）、喜欢（{{ lists.likes.value.length }}）、浏览历史（{{
            lists.history.value.length
          }}）、刷到过（{{ lists.seen.value.length }}）、各频道看到的位置和频道设置保存成一个文件。
        </p>
        <button class="btn btn-primary" @click="downloadBackup">下载备份文件</button>
      </div>
      <div class="card">
        <h2>恢复</h2>
        <p>选择之前下载的备份文件，然后选择合并还是覆盖。</p>
        <label class="btn btn-ghost file">
          选择备份文件
          <input ref="fileInput" type="file" accept="application/json,.json" @change="pickFile" />
        </label>
        <p v-if="restoreError" class="error">{{ restoreError }}</p>
        <p v-if="restored" class="ok">{{ restored }}</p>
        <div v-if="pending" class="pending">
          <p>
            备份于 {{ exportedAt(pending) }}：关注 {{ pending.follows.length }}、收藏 {{ pending.saves.length }}、喜欢
            {{ pending.likes.length }}、历史 {{ pending.history.length }}。
          </p>
          <div class="actions">
            <button class="btn btn-primary" @click="applyRestore('merge')">合并到现有数据</button>
            <button class="btn btn-ghost" @click="applyRestore('replace')">用备份覆盖</button>
            <button class="link" @click="pending = null">取消</button>
          </div>
          <p class="note">合并：保留现在的记录，加入备份里没有的；频道设置不变。覆盖：完全换成备份里的内容。</p>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.head {
  padding: 16px 4px 8px;
}
h1 {
  margin: 0;
  font-size: 22px;
}
.note {
  margin: 4px 0 12px;
  color: var(--text-3);
  font-size: 13px;
}
.bar {
  display: flex;
  align-items: center;
  gap: 16px;
  margin: 4px 4px 12px;
  font-size: 13px;
}
.spacer {
  flex: 1;
}
.filter {
  width: 220px;
  height: 32px;
  padding: 0 14px;
  border: 0;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  outline: none;
}
.link {
  color: var(--link);
  font-size: 13px;
}
.danger {
  color: var(--accent);
}
.switch {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-2);
}
.switch input {
  accent-color: var(--accent);
}
.empty {
  margin: 32px 0;
  color: var(--text-2);
  text-align: center;
}
.blog-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(260px, 100%), 1fr));
  gap: 10px;
  margin: 8px 0 32px;
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
.backup {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(300px, 100%), 1fr));
  gap: 16px;
  max-width: 880px;
  margin: 8px 0 48px;
}
.card {
  padding: 16px 18px;
  border-radius: 14px;
  background: var(--surface);
}
.card h2 {
  margin: 0 0 6px;
  font-size: 16px;
}
.card p {
  margin: 0 0 12px;
  color: var(--text-2);
  font-size: 14px;
}
.file {
  position: relative;
  overflow: hidden;
  background: var(--bg);
}
.file input {
  position: absolute;
  inset: 0;
  opacity: 0;
  cursor: pointer;
}
.pending {
  margin-top: 12px;
}
.actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.error {
  margin-top: 10px !important;
  color: var(--accent) !important;
}
.ok {
  margin-top: 10px !important;
  color: #2e9d5b !important;
}
.spinner {
  display: inline-block;
  width: 22px;
  height: 22px;
  border: 2px solid var(--line);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (max-width: 639px) {
  .head {
    padding: 12px 6px 4px;
  }
  .blog-list {
    grid-template-columns: minmax(0, 1fr);
  }
  .bar {
    flex-wrap: wrap;
    margin: 4px 6px 10px;
  }
  .filter {
    flex: 1;
    width: auto;
    min-width: 0;
  }
}
</style>
