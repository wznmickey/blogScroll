<script setup lang="ts">
// 设置 → 诊断信息: what helps find out why something does not work here.
// This build, how the site runs and what its server offers, syncing, how
// full the browser's storage is, and the problems noted lately
// (diagnostics.ts). 「复制全部」 puts it on the clipboard as plain text.
import { computed, ref } from 'vue'
import { isStatic } from '../api'
import { clearProblems, problems } from '../diagnostics'
import { relativeTime } from '../format'
import { entries, LIST_LABELS, LIST_NAMES } from '../library'
import { capabilities, loadCapabilities } from '../server'
import { ownFeeds } from '../sources'
import { syncStatus, syncSupported } from '../sync'

/** Commit and time of the build (vite.config.ts). */
declare const __BUILD__: string

void loadCapabilities()

const CAPABILITIES: [keyof NonNullable<typeof capabilities.value>, string][] = [
  ['sync', '同步'],
  ['fetch', '代读 RSS'],
  ['readers', '抓取读者的订阅'],
  ['refresh', '手动刷新博客'],
  ['stats', '匿名统计'],
]

/** What each key in localStorage takes, biggest first (null: storage not readable). */
function storageUse(): { key: string; chars: number }[] | null {
  try {
    const out = []
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i) ?? ''
      out.push({ key, chars: key.length + (localStorage.getItem(key)?.length ?? 0) })
    }
    return out.sort((a, b) => b.chars - a.chars)
  } catch {
    return null
  }
}
const storage = ref(storageUse())
// Browsers count two bytes a character, and allow about 5 MB a site.
const kb = (chars: number) => `${Math.ceil((chars * 2) / 1024)} KB`

const facts = computed(() => {
  const c = capabilities.value
  const s = syncStatus.value
  const used = storage.value
  return [
    ['版本', __BUILD__],
    ['网址', `${location.origin}${import.meta.env.BASE_URL}`],
    ['运行方式', isStatic ? '静态网站（数据随发布更新）' : '本站服务器'],
    ['服务器提供', c ? CAPABILITIES.filter(([k]) => c[k]).map(([, l]) => l).join('、') || '都不提供' : '正在检查…'],
    [
      '同步',
      !s.on
        ? syncSupported()
          ? '没有开启'
          : '这个浏览器不支持'
        : `已开启，${s.lastSync ? `上次成功 ${relativeTime(s.lastSync / 1000)}` : '还没成功过'}${s.error ? `；出错：${s.error}` : ''}`,
    ],
    [
      '本机数据',
      `${LIST_NAMES.map((n) => `${LIST_LABELS[n]} ${entries(n).value.length}`).join('、')}；自己的订阅 ${ownFeeds.value.length} 个`,
    ],
    [
      '本机存储',
      used
        ? `约 ${kb(used.reduce((n, x) => n + x.chars, 0))}（上限约 5 MB）；最大的：${used
            .slice(0, 3)
            .map((x) => `${x.key} ${kb(x.chars)}`)
            .join('、')}`
        : '读不到：浏览器不让这个网站保存数据',
    ],
    ['浏览器', navigator.userAgent],
  ] as const
})

function report(): string {
  storage.value = storageUse()
  const time = (ms: number) => new Date(ms).toISOString().replace('T', ' ').slice(0, 19)
  return [
    `博客瀑布 · 诊断信息（${time(Date.now())} UTC）`,
    ...facts.value.map(([k, v]) => `${k}：${v}`),
    '',
    problems.value.length ? '最近的问题（UTC）：' : '最近没有记下问题。',
    ...problems.value.map((p) => `${time(p.at)}  ${p.where}：${p.message}${p.count > 1 ? `（${p.count} 次）` : ''}`),
  ].join('\n')
}

const copied = ref(false)
const manual = ref('')
async function copy() {
  const text = report()
  try {
    await navigator.clipboard.writeText(text)
    copied.value = true
    setTimeout(() => (copied.value = false), 2000)
  } catch {
    // No clipboard here (plain http, an old browser): select it by hand.
    manual.value = text
  }
}
</script>

<template>
  <p class="hint">
    网站有地方不对劲时，把这里的内容复制给站长，比描述现象更容易找到原因。只存在这台浏览器里，不会自动发送；包括浏览器型号和版本。
  </p>
  <dl class="facts">
    <template v-for="[k, v] in facts" :key="k">
      <dt>{{ k }}</dt>
      <dd>{{ v }}</dd>
    </template>
  </dl>
  <h3>最近的问题</h3>
  <p v-if="!problems.length" class="hint">没有记下问题。</p>
  <ul v-else class="problems">
    <li v-for="p in problems" :key="`${p.where}|${p.message}`">
      <span class="when">{{ relativeTime(p.at / 1000) }}</span>
      <b>{{ p.where }}</b>
      {{ p.message }}<span v-if="p.count > 1" class="count">（{{ p.count }} 次）</span>
    </li>
  </ul>
  <p class="actions">
    <button class="btn btn-primary" @click="copy">{{ copied ? '已复制' : '复制全部' }}</button>
    <button class="btn btn-ghost" :disabled="!problems.length" @click="clearProblems">清空问题记录</button>
  </p>
  <textarea v-if="manual" class="manual" readonly rows="10" :value="manual" @focus="($event.target as HTMLTextAreaElement).select()" />
</template>

<style scoped>
.hint {
  margin: 4px 0;
  color: var(--text-3);
  font-size: 13px;
}
.facts {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 4px 12px;
  margin: 10px 0;
  font-size: 13px;
}
.facts dt {
  color: var(--text-3);
}
.facts dd {
  margin: 0;
  overflow-wrap: anywhere;
}
.problems {
  margin: 6px 0;
  padding-left: 18px;
  font-size: 13px;
}
.problems li {
  margin: 4px 0;
  overflow-wrap: anywhere;
}
.when,
.count {
  color: var(--text-3);
}
.when {
  margin-right: 6px;
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: 10px 0;
}
.manual {
  width: 100%;
  box-sizing: border-box;
  font: 12px/1.5 ui-monospace, monospace;
}
</style>
