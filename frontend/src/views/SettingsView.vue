<script setup lang="ts">
// The reader's settings: what 推荐 weighs and why, what to leave out (pictures,
// excerpts, comments, adult content, blocked bloggers, sites and words),
// which directories, and how the page looks. All kept in this browser
// (options.ts, channels.ts) and included in backups.
import { blogPath } from '../format'
import { computed, ref, watch } from 'vue'
import { api, categoryName, meta } from '../api'
import { COLUMN_CHOICES, HIDE_CHOICES, prefs } from '../channels'
import { follows } from '../follow'
import { recentIds } from '../library'
import {
  DEFAULT_WEIGHTS,
  hostKey,
  options,
  PIN_CHOICES,
  sameWeights,
  TEXT_SIZES,
  WEIGHT_INFO,
} from '../options'
import type { InterestProfile } from '../types'

document.title = '设置'

const isDefault = computed(() => sameWeights(options.value.weights, DEFAULT_WEIGHTS))
const resetWeights = () => (options.value.weights = { ...DEFAULT_WEIGHTS })

// What 推荐 makes of this reader now: the same signals the home page sends.
const profile = ref<InterestProfile | null>(null)
const signals = computed(() => ({
  follow: [...follows.value],
  like: recentIds('likes', 100),
  save: recentIds('saves', 100),
  read: recentIds('history', 50),
}))
const hasSignals = computed(() => Object.values(signals.value).some((l) => l.length))
watch(
  signals,
  async (s) => {
    profile.value = hasSignals.value ? await api.interest(s).catch(() => null) : null
  },
  { immediate: true },
)
const topTerms = computed(() => Object.entries(profile.value?.terms ?? {}).slice(0, 16))
const topCategories = computed(() =>
  Object.entries(profile.value?.categories ?? {})
    .filter(([id]) => !id.includes('/'))
    .sort((a, b) => b[1] - a[1])
    .slice(0, 8),
)

// Lists typed one at a time.
const hostText = ref('')
const wordText = ref('')
function addHost() {
  const h = hostKey(hostText.value)
  if (h && !options.value.blockHosts.includes(h)) options.value.blockHosts = [...options.value.blockHosts, h]
  hostText.value = ''
}
function addWord() {
  const w = wordText.value.trim().replace(/^#/, '')
  if (w && !options.value.blockWords.includes(w)) options.value.blockWords = [...options.value.blockWords, w]
  wordText.value = ''
}
const without = <T,>(list: T[], x: T) => list.filter((y) => y !== x)

// Sources: every directory unless some are unticked.
const directories = computed(() => meta.value?.directories ?? [])
const directoryOn = (id: string) => options.value.directories?.includes(id) ?? true
function toggleDirectory(id: string) {
  const on = directories.value.map((d) => d.id).filter((d) => (d === id ? !directoryOn(d) : directoryOn(d)))
  options.value.directories = on.length === directories.value.length ? null : on
}
</script>

<template>
  <div class="container">
    <article class="settings">
      <h1>设置</h1>
      <p class="lead">只保存在这台设备的浏览器里，换设备可以用<RouterLink to="/me?tab=backup">「我的 → 备份与恢复」</RouterLink>带走。</p>

      <section id="recommend">
        <header class="head">
          <h2>推荐的权重</h2>
          <button v-if="!isDefault" class="link" @click="resetWeights">恢复默认</button>
        </header>
        <p class="hint">
          「推荐」按发布时间排，再按下面几项把文章提前或推后若干天；「反推荐」把兴趣、分类和博主这几项反过来。打开推荐里的文章，可以看到它为什么排在那里。
        </p>
        <div v-for="w in WEIGHT_INFO" :key="w.key" class="weight">
          <label :for="`w-${w.key}`">
            <span class="name">{{ w.label }}</span>
            <span class="value">{{ options.weights[w.key] }}{{ w.key === 'perBlog' ? ' 篇' : ' 天' }}</span>
          </label>
          <input
            :id="`w-${w.key}`"
            v-model.number="options.weights[w.key]"
            type="range"
            :min="w.key === 'perBlog' ? 1 : 0"
            :max="w.max"
            :step="w.step"
          />
          <p class="hint">{{ w.hint }}（默认 {{ DEFAULT_WEIGHTS[w.key] }}）</p>
        </div>

        <h3>推荐眼中的你</h3>
        <p v-if="!hasSignals" class="hint">还没有关注、喜欢、收藏或读过的文章，「推荐」现在只看新鲜度和随机打散。</p>
        <template v-else-if="profile">
          <p class="hint">来自你关注的博主、喜欢和收藏的文章、最近读过的文章；越靠前越常见。</p>
          <p v-if="topCategories.length" class="chips">
            <span v-for="[id, share] in topCategories" :key="id" class="chip">
              {{ categoryName(id) }} <small>{{ Math.round(share * 100) }}%</small>
            </span>
          </p>
          <p class="chips">
            <span v-for="[term, w] in topTerms" :key="term" class="chip term" :style="{ '--w': w }">{{ term }}</span>
          </p>
        </template>
      </section>

      <section id="filter">
        <h2>过滤</h2>
        <h3>只看</h3>
        <p class="checks">
          <label v-for="c in HIDE_CHOICES" :key="c.key" class="check" :title="c.title">
            <input v-model="prefs.hide[c.key]" type="checkbox" /> {{ c.label }}
            <small>{{ c.title }}</small>
          </label>
        </p>
        <label class="check">
          <input v-model="options.showNsfw" type="checkbox" /> 显示成人内容（R18 / NSFW）
          <small>标题或作者标签里带有 R18、NSFW 等字样的文章，默认不显示</small>
        </label>

        <h3>屏蔽的博主</h3>
        <p v-if="!options.blockBlogs.length" class="hint">在博主页点「屏蔽」，他的文章就不会出现在推荐、最新和搜索里。</p>
        <p v-else class="chips">
          <span v-for="b in options.blockBlogs" :key="b.id" class="chip removable">
            <RouterLink :to="blogPath(b)">{{ b.name }}</RouterLink>
            <button :aria-label="`取消屏蔽 ${b.name}`" @click="options.blockBlogs = options.blockBlogs.filter((x) => x.id !== b.id)">✕</button>
          </span>
        </p>

        <h3>屏蔽的网站</h3>
        <form class="add" @submit.prevent="addHost">
          <input v-model="hostText" placeholder="example.com（也包括它的子域名）" />
          <button class="btn btn-ghost" :disabled="!hostText.trim()">添加</button>
        </form>
        <p v-if="options.blockHosts.length" class="chips">
          <span v-for="h in options.blockHosts" :key="h" class="chip removable">
            {{ h }}
            <button :aria-label="`取消屏蔽 ${h}`" @click="options.blockHosts = without(options.blockHosts, h)">✕</button>
          </span>
        </p>

        <h3>屏蔽的关键词和标签</h3>
        <form class="add" @submit.prevent="addWord">
          <input v-model="wordText" placeholder="出现在标题、摘要或标签里就不显示" />
          <button class="btn btn-ghost" :disabled="!wordText.trim()">添加</button>
        </form>
        <p v-if="options.blockWords.length" class="chips">
          <span v-for="w in options.blockWords" :key="w" class="chip removable">
            {{ w }}
            <button :aria-label="`取消屏蔽 ${w}`" @click="options.blockWords = without(options.blockWords, w)">✕</button>
          </span>
        </p>
      </section>

      <section id="sources">
        <h2>来源</h2>
        <p class="hint">本站从这些博客目录和订阅列表收录博客；取消勾选的来源，其博客不再出现在推荐、最新和搜索里。</p>
        <p class="checks">
          <label v-for="d in directories" :key="d.id" class="check">
            <input type="checkbox" :checked="directoryOn(d.id)" @change="toggleDirectory(d.id)" /> {{ d.name }}
            <a v-if="d.homepage" :href="d.homepage" target="_blank" rel="noopener"><small>{{ d.homepage }}</small></a>
          </label>
        </p>
      </section>

      <section id="display">
        <h2>显示</h2>
        <h3>每行几列</h3>
        <p class="choices" role="group" aria-label="每行几列">
          <button
            v-for="n in COLUMN_CHOICES"
            :key="String(n)"
            :class="{ on: prefs.columns === n }"
            :aria-pressed="prefs.columns === n"
            @click="prefs.columns = n"
          >
            {{ n ?? '自动' }}
          </button>
        </p>
        <h3>字号</h3>
        <p class="choices" role="group" aria-label="字号">
          <button
            v-for="t in TEXT_SIZES"
            :key="t.value"
            :class="{ on: options.textSize === t.value }"
            :aria-pressed="options.textSize === t.value"
            @click="options.textSize = t.value"
          >
            {{ t.label }}
          </button>
        </p>
        <h3>最多置顶几个频道</h3>
        <p class="choices" role="group" aria-label="最多置顶几个频道">
          <button
            v-for="n in PIN_CHOICES"
            :key="n"
            :class="{ on: options.maxPinned === n }"
            :aria-pressed="options.maxPinned === n"
            @click="options.maxPinned = n"
          >
            {{ n }}
          </button>
        </p>
      </section>

      <p class="footer"><RouterLink to="/about">关于本站</RouterLink></p>
    </article>
  </div>
</template>

<style scoped>
.settings {
  max-width: 720px;
  margin: 24px auto 64px;
  padding: 0 8px;
}
h1 {
  margin: 0 0 6px;
  font-size: 26px;
}
.lead,
.hint {
  margin: 4px 0;
  color: var(--text-3);
  font-size: 13px;
}
.lead a,
.footer a,
.check a {
  color: var(--link);
}
section {
  margin-top: 28px;
  padding: 16px 18px;
  border-radius: 14px;
  background: var(--surface);
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
h2 {
  margin: 0 0 6px;
  font-size: 18px;
}
h3 {
  margin: 18px 0 6px;
  font-size: 14px;
}
.link {
  color: var(--link);
  font-size: 13px;
}
.weight {
  margin: 14px 0 0;
}
.weight label {
  display: flex;
  justify-content: space-between;
  font-size: 14px;
}
.weight .value {
  color: var(--accent);
  font-variant-numeric: tabular-nums;
}
.weight input {
  width: 100%;
  accent-color: var(--accent);
}
.weight .hint {
  margin-top: 0;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 8px 0 0;
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 10px;
  border-radius: 999px;
  background: var(--bg);
  color: var(--text-2);
  font-size: 13px;
}
.chip small {
  color: var(--text-3);
}
/* Stronger terms of the profile read darker. */
.chip.term {
  color: color-mix(in srgb, var(--text) calc(40% + var(--w) * 200%), var(--text-3));
}
.chip.removable button {
  color: var(--text-3);
  font-size: 11px;
}
.chip.removable button:hover {
  color: var(--text);
}
.checks {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 6px 0;
}
.check {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 4px 8px;
  margin: 6px 0;
  font-size: 14px;
  cursor: pointer;
}
.check input {
  accent-color: var(--accent);
}
.check small {
  flex-basis: 100%;
  padding-left: 22px;
  color: var(--text-3);
  font-size: 12px;
}
.add {
  display: flex;
  gap: 8px;
}
.add input {
  flex: 1;
  min-width: 0;
  height: 34px;
  padding: 0 12px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--bg);
  color: var(--text);
}
.choices {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 6px 0;
}
.choices button {
  min-width: 40px;
  height: 30px;
  padding: 0 12px;
  border-radius: 999px;
  background: var(--bg);
  color: var(--text-2);
  font-size: 13px;
}
.choices button.on {
  background: var(--accent);
  color: #fff;
  font-weight: 600;
}
.footer {
  margin: 28px 0 0;
  text-align: center;
  font-size: 13px;
}
</style>
