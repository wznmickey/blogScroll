<script setup lang="ts">
// The reader's settings: syncing them between devices, what 推荐 weighs and
// why, what to leave out (pictures, excerpts, comments, adult content,
// blocked bloggers, sites and words), where posts come from (the site's
// directories, and feeds of the reader's own), and how the page looks. All
// kept in this browser (options.ts, channels.ts, sources.ts), synced
// (sync.ts) and included in backups.
import { blogPath, relativeTime } from '../format'
import { computed, ref, watch } from 'vue'
import { api, categoryName, meta, tagsInfo } from '../api'
import { addChannel, COLUMN_CHOICES, HIDE_CHOICES, prefs } from '../channels'
import { follows, isFollowing, toggleFollow } from '../follow'
import { recentIds } from '../library'
import { capabilities } from '../server'
import {
  addOwn,
  crawled,
  fromFile,
  lookUp,
  ownBlogId,
  ownSources,
  readState,
  removeOwn,
  siteKey,
  type Found,
  type OwnSource,
} from '../sources'
import {
  deleteSync,
  joinOffer,
  newSyncCode,
  pairingLink,
  startSync,
  stopSync,
  syncCode,
  syncNow,
  syncStatus,
  syncSupported,
} from '../sync'
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

// 多设备同步
const codeText = ref('')
const syncError = ref('')
const codeShown = ref(false)
const qr = ref('')
const copied = ref(false)
const code = computed(() => (syncStatus.value.on ? syncCode() : null))
const host = location.host
async function begin(c: string) {
  syncError.value = ''
  try {
    await startSync(c)
    codeText.value = ''
  } catch (e) {
    syncError.value = (e as Error).message
  }
}
watch(
  [code, codeShown],
  async ([c, shown]) => {
    qr.value = ''
    if (!c || !shown) return
    const { renderSVG } = await import('uqr')
    qr.value = renderSVG(pairingLink(c), { border: 2, pixelSize: 4 })
  },
  { immediate: true },
)
async function copyLink() {
  if (!code.value) return
  await navigator.clipboard.writeText(pairingLink(code.value))
  copied.value = true
  setTimeout(() => (copied.value = false), 2000)
}
function stop() {
  if (confirm('这台设备停止同步？数据留在这台设备上，别的设备照常同步。')) stopSync()
}
async function removeAll() {
  if (!confirm('删除服务器上的同步数据？所有设备都会停止同步，各自的数据留在各自的设备上。')) return
  try {
    await deleteSync()
  } catch (e) {
    syncError.value = (e as Error).message
  }
}
const syncText = computed(() => {
  const s = syncStatus.value
  if (s.busy) return '同步中…'
  if (s.error) return `没能同步：${s.error}`
  return s.lastSync ? `已同步 · ${relativeTime(s.lastSync / 1000)}` : '已开启'
})

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

// Sources of the reader's own: a feed, a page with one, or a list (OPML).
const sourceText = ref('')
const adding = ref(false)
const sourceNote = ref<{ text: string; bad?: boolean } | null>(null)
/** Blogs the site has already: followed instead. */
async function onSite(found: Found) {
  const key = found.source.kind === 'feed' ? siteKey(found.source.homepage) : null
  return key ? api.blogByKey(key).catch(() => null) : null
}
async function add(get: () => Promise<Found>) {
  adding.value = true
  sourceNote.value = null
  try {
    const found = await get()
    const blog = await onSite(found)
    if (blog) {
      if (!isFollowing(blog.id)) toggleFollow(blog.id)
      sourceNote.value = { text: `本站已经收录了「${blog.name}」，已为你关注，它的文章在「关注」频道里。` }
    } else if (ownSources.value.some((s) => s.url === found.source.url)) {
      sourceNote.value = { text: '已经添加过了' }
    } else {
      addOwn(found)
      addChannel('sources', tagsInfo.value)
      const n = found.source.feeds?.length
      sourceNote.value = { text: `已添加「${found.source.title}」${n ? `（${n} 个订阅）` : ''}，文章在首页的「订阅」频道里。` }
    }
    sourceText.value = ''
  } catch (e) {
    sourceNote.value = { text: (e as Error).message || '没能读取', bad: true }
  } finally {
    adding.value = false
  }
}
const addAddress = () => add(() => lookUp(sourceText.value))
function addFile(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (file) void add(() => fromFile(file))
}
function readText(s: OwnSource): { text: string; bad: boolean } | null {
  if (s.kind === 'feed' && crawled.value[s.url]) return { text: '本站在抓取', bad: false }
  if (s.kind === 'list' && s.feeds?.length) {
    const n = s.feeds.filter((f) => crawled.value[f.url]).length
    if (n) return { text: `本站在抓取其中 ${n} 个`, bad: false }
  }
  if (s.url.startsWith('file:')) return null
  const r = readState(s.url)
  if (!r) return { text: '还没读过', bad: false }
  if (r.error) return { text: `上次读取失败：${r.error}`, bad: true }
  return { text: `${relativeTime(r.ok / 1000)}读过${s.kind === 'feed' ? ` · ${r.count} 篇` : ''}`, bad: false }
}
/** On GitHub: an issue asking the site to list it (or any blog). */
function suggestLink(s?: OwnSource): string | null {
  const repo = meta.value?.repo?.url
  if (!repo?.startsWith('https://github.com/')) return null
  const title = s ? `收录建议：${s.title}` : '收录建议'
  const body = s ? `RSS：${s.url}\n主页：${s.homepage}\n\n（来自「设置 → 来源」）` : 'RSS 或博客地址：\n'
  return `${repo.replace(/\/$/, '')}/issues/new?title=${encodeURIComponent(title)}&body=${encodeURIComponent(body)}`
}

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
      <p class="lead">
        保存在这台设备的浏览器里。开启下面的「多设备同步」，别的设备会自动保持一致；也可以用<RouterLink
          to="/me?tab=backup"
          >「我的 → 备份与恢复」</RouterLink
        >手动带走。
      </p>

      <section id="sync">
        <h2>多设备同步</h2>
        <p class="hint">
          关注、喜欢、收藏、历史、刷到过、看到哪里、频道栏、这些设置和你添加的来源，在用同一个同步码的设备之间自动保持一致。数据先在浏览器里加密再上传，服务器看不到内容；不需要注册。
        </p>
        <p class="hint">
          同步码存在当前这个网址（{{ host }}）的服务器上：不同的网址是各自独立的两套，比如本地服务器和公开网站之间不互通；两台设备要打开同一个网址。
        </p>
        <p v-if="!capabilities" class="hint">正在检查这里的服务器…</p>
        <p v-else-if="!capabilities.sync" class="hint">这里的服务器不提供同步（例如放在 GitHub Pages 上的静态版）。</p>
        <p v-else-if="!syncSupported()" class="hint warn">
          这个浏览器不能在本地加密，所以不能同步：网址要是 https（或本机的 localhost / 127.0.0.1，用局域网 IP 打开的 http 不行），浏览器也要够新（iOS 要 16.4 以上）。
        </p>
        <template v-else-if="!syncStatus.on">
          <p v-if="syncStatus.gone" class="hint warn">另一台设备删除了服务器上的同步数据，同步已停止；这台设备上的数据都还在。</p>
          <div v-if="joinOffer" class="offer">
            <p>
              用同步码 <code>{{ joinOffer }}</code> 让这台设备加入同步？这里的数据会和其他设备的合并，不会丢。
            </p>
            <p class="row">
              <button class="btn btn-primary" @click="begin(joinOffer)">加入同步</button>
              <button class="btn btn-ghost" @click="joinOffer = null">不用了</button>
            </p>
          </div>
          <p class="row"><button class="btn btn-primary" @click="begin(newSyncCode())">开启同步</button></p>
          <form class="add" @submit.prevent="begin(codeText)">
            <input v-model="codeText" placeholder="已在别的设备开启？输入那里的同步码" autocomplete="off" spellcheck="false" />
            <button class="btn btn-ghost" :disabled="!codeText.trim()">加入</button>
          </form>
        </template>
        <template v-else>
          <p class="state" :class="{ warn: syncStatus.error }">{{ syncText }}</p>
          <p class="hint">
            在另一台设备上打开配对链接（或用手机扫码），或在那里的「设置 → 多设备同步」输入同步码。同步码就是钥匙，不要给别人。
          </p>
          <div class="pair">
            <!-- Made here from the code (uqr): no request anywhere. -->
            <div v-if="qr" class="qr" aria-label="配对二维码" v-html="qr" />
            <div class="pair-text">
              <p>
                <code class="code">{{ codeShown ? code : '••••-••••-••••-••••-••••-••••' }}</code>
                <button class="link" @click="codeShown = !codeShown">{{ codeShown ? '隐藏' : '显示同步码和二维码' }}</button>
              </p>
              <p class="row">
                <button class="btn btn-ghost" @click="copyLink">{{ copied ? '已复制' : '复制配对链接' }}</button>
                <button class="btn btn-ghost" :disabled="syncStatus.busy" @click="syncNow()">立即同步</button>
              </p>
            </div>
          </div>
          <p class="row small">
            <button class="link" @click="stop">这台设备停止同步</button>
            <button class="link danger" @click="removeAll">删除服务器上的数据（所有设备停止）</button>
          </p>
        </template>
        <p v-if="syncError" class="hint warn">{{ syncError }}</p>
      </section>

      <section id="recommend">
        <header class="head">
          <h2>推荐的权重</h2>
          <button v-if="!isDefault" class="link" @click="resetWeights">恢复默认</button>
        </header>
        <p class="hint">
          「推荐」按发布时间排，再按下面几项把文章提前或推后若干天；「反推荐」把兴趣、分类和博主这几项反过来。打开推荐里的文章，可以看到它为什么排在那里。
        </p>
        <div v-for="w in WEIGHT_INFO.filter((x) => x.key !== 'heat' || options.useOthers)" :key="w.key" class="weight">
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

      <section id="stats">
        <h2>统计与热度</h2>
        <label class="check">
          <input v-model="options.sendStats" type="checkbox" /> 发送匿名使用统计
          <small>
            访问、停留时长、打开和读原文、喜欢、收藏、关注、搜索词，以及每天一次用了哪些设置。用来算热度、「大家在看」和「其他读者在看」的推荐。只带一个这台浏览器自己的随机编号（不是同步码，不随同步走），用来把同一位访客、同一个订阅的读者只算一次；服务器存的是它的哈希和每天的总数，不记 IP，也不知道你是谁。关掉后不再发送，你添加的来源也不再计入订阅人数。
          </small>
        </label>
        <label class="check">
          <input v-model="options.useOthers" type="checkbox" /> 推荐时参考其他读者在看什么
          <small>其他读者打开、喜欢、收藏、读得久的文章在「推荐」里往前提（多少天见上面的「其他读者在看」），「反推荐」里往后放。</small>
        </label>
        <label class="check">
          <input v-model="options.showHeat" type="checkbox" /> 显示热度
          <small>卡片和文章上显示 🔥 热度：最近 30 天里，比多少百分比的文章更受读者关注。</small>
        </label>
        <p class="hint"><RouterLink to="/trending">看看大家在看什么 →</RouterLink></p>
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
        <h3>本站的来源 <small>大家共用</small></h3>
        <p class="hint">本站从这些博客目录和订阅列表收录博客；取消勾选的来源，其博客不再出现在推荐、最新和搜索里。</p>
        <p class="checks">
          <label v-for="d in directories" :key="d.id" class="check">
            <input type="checkbox" :checked="directoryOn(d.id)" @change="toggleDirectory(d.id)" /> {{ d.name }}
            <a v-if="d.homepage" :href="d.homepage" target="_blank" rel="noopener"><small>{{ d.homepage }}</small></a>
          </label>
        </p>
        <p v-if="suggestLink()" class="hint">
          希望本站收录某个博客？<a :href="suggestLink()!" target="_blank" rel="noopener">在 GitHub 上提建议</a>
        </p>

        <h3>我添加的来源 <small>只有你看得到</small></h3>
        <p class="hint">
          RSS / Atom 地址、博客首页（会找到它的 RSS），或 OPML 订阅列表的地址或文件。本站已经收录的博客，会直接帮你关注。
          <template v-if="capabilities?.readers">
            其余的由本站像收录的博客一样抓取：保留全部历史、自动分类，也出现在推荐、最新和分类里，但只有你看得到（服务器只记下这些 RSS 地址，不知道是谁添加的）。在本站抓到之前，先由你的浏览器经本站读取最新的几篇，放在首页的「订阅」频道。
          </template>
          <template v-else>文章出现在首页的「订阅」频道；由本站的服务器代为读取，每个最多半小时一次。</template>
        </p>
        <template v-if="capabilities?.fetch">
          <form class="add" @submit.prevent="addAddress">
            <input v-model="sourceText" placeholder="https://example.com/feed.xml" :disabled="adding" spellcheck="false" />
            <button class="btn btn-ghost" :disabled="!sourceText.trim() || adding">{{ adding ? '读取中…' : '添加' }}</button>
          </form>
          <p class="row small">
            <label class="link file">
              导入 OPML 文件
              <input type="file" accept=".opml,.xml,text/xml,application/xml,text/x-opml" hidden @change="addFile" />
            </label>
          </p>
          <p v-if="sourceNote" class="hint" :class="{ warn: sourceNote.bad }">{{ sourceNote.text }}</p>
          <ul v-if="ownSources.length" class="own">
            <li v-for="s in ownSources" :key="s.url">
              <div class="own-main">
                <RouterLink v-if="s.kind === 'feed'" :to="`/blog/${ownBlogId(s.url)}`" class="own-title">{{ s.title }}</RouterLink>
                <span v-else class="own-title">列表：{{ s.title }} <small>{{ s.feeds?.length ?? 0 }} 个订阅</small></span>
                <small class="own-url">{{ s.url.startsWith('file:') ? `从文件 ${s.url.slice(5)} 导入` : s.url }}</small>
                <small v-if="readText(s)" :class="{ warn: readText(s)!.bad }">{{ readText(s)!.text }}</small>
              </div>
              <a v-if="s.kind === 'feed' && suggestLink(s)" :href="suggestLink(s)!" target="_blank" rel="noopener" class="link">推荐给本站</a>
              <button class="link" @click="removeOwn(s.url)">移除</button>
            </li>
          </ul>
        </template>
        <p v-else-if="capabilities" class="hint">这里的服务器不能代为读取 RSS（例如放在 GitHub Pages 上的静态版），所以暂时不能添加。</p>
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
.warn {
  color: var(--accent);
}
.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 14px;
  margin: 10px 0;
}
.row.small {
  font-size: 13px;
}
.link.danger {
  color: var(--accent);
}
.offer {
  margin: 10px 0;
  padding: 10px 14px;
  border-radius: 10px;
  background: var(--bg);
  font-size: 14px;
}
.state {
  margin: 6px 0;
  font-size: 14px;
  font-weight: 600;
}
.pair {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px;
  margin: 10px 0;
}
.qr {
  flex: none;
  width: 148px;
  line-height: 0;
}
.qr :deep(svg) {
  width: 100%;
  height: auto;
  border-radius: 8px;
}
.pair-text {
  min-width: 0;
}
.code {
  margin-right: 8px;
  font-size: 15px;
  letter-spacing: 0.04em;
  word-break: break-all;
}
h3 small {
  margin-left: 6px;
  color: var(--text-3);
  font-weight: 400;
  font-size: 12px;
}
.file {
  cursor: pointer;
}
.own {
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
}
.own li {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 0;
  border-top: 1px solid var(--line);
  font-size: 13px;
}
.own-main {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}
.own-title {
  color: var(--text);
  font-size: 14px;
  font-weight: 600;
}
.own-main small {
  overflow: hidden;
  color: var(--text-3);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.own-main small.warn {
  color: var(--accent);
}
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
