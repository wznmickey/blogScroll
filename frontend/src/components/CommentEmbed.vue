<script setup lang="ts">
// The blog's own comment widget, for blogs whose authors allowed embedding.
// Comments are read from and written to the original blog's comment service.
// Scripts come only from these pinned CDN builds, never from the blog itself,
// and load only when the comment area scrolls into view.
import { onBeforeUnmount, onMounted, ref } from 'vue'
import type { EmbedConfig } from '../types'

const props = defineProps<{ config: EmbedConfig }>()

const CDN = {
  twikoo: 'https://cdn.jsdelivr.net/npm/twikoo@2.0.17/dist/twikoo.all.min.js',
  waline: 'https://cdn.jsdelivr.net/npm/@waline/client@3.16.2/dist/waline.js',
  walineCss: 'https://cdn.jsdelivr.net/npm/@waline/client@3.16.2/dist/waline.css',
  artalk: 'https://cdn.jsdelivr.net/npm/artalk@2.10.1/dist/Artalk.iife.js',
  artalkCss: 'https://cdn.jsdelivr.net/npm/artalk@2.10.1/dist/Artalk.css',
  giscus: 'https://giscus.app/client.js',
  utterances: 'https://utteranc.es/client.js',
}

const root = ref<HTMLElement>()
const box = ref<HTMLElement>()
const state = ref<'waiting' | 'loading' | 'ready' | 'error'>('waiting')
const boxId = `comments-${Math.random().toString(36).slice(2)}`
let destroy: (() => void) | undefined
let observer: IntersectionObserver | undefined

const loaded = new Map<string, Promise<void>>()
function loadScript(src: string): Promise<void> {
  let p = loaded.get(src)
  if (!p) {
    p = new Promise<void>((resolve, reject) => {
      const s = document.createElement('script')
      s.src = src
      s.async = true
      s.onload = () => resolve()
      s.onerror = () => reject(new Error(`failed to load ${src}`))
      document.head.appendChild(s)
    })
    p.catch(() => loaded.delete(src))
    loaded.set(src, p)
  }
  return p
}
function loadCss(href: string) {
  if (document.querySelector(`link[href="${href}"]`)) return
  const l = document.createElement('link')
  l.rel = 'stylesheet'
  l.href = href
  document.head.appendChild(l)
}
function scriptWidget(src: string, attrs: Record<string, string | null>) {
  const s = document.createElement('script')
  s.src = src
  s.async = true
  s.crossOrigin = 'anonymous'
  for (const [k, v] of Object.entries(attrs)) if (v) s.setAttribute(k, v)
  box.value!.appendChild(s)
}

type Destroyable = { destroy?: () => void } | undefined
type Globals = {
  twikoo?: { init: (o: object) => Promise<unknown> }
  Artalk?: { init: (o: object) => Destroyable }
}
const dark = () => window.matchMedia('(prefers-color-scheme: dark)').matches

async function mount() {
  const c = props.config
  const g = window as unknown as Globals
  state.value = 'loading'
  try {
    switch (c.kind) {
      case 'twikoo':
        await loadScript(CDN.twikoo)
        await g.twikoo!.init({ envId: c.env_id, el: `#${boxId}`, path: c.path, lang: 'zh-CN' })
        break
      case 'waline': {
        loadCss(CDN.walineCss)
        const mod = (await import(/* @vite-ignore */ CDN.waline)) as {
          init: (o: object) => Destroyable
        }
        const w = mod.init({ el: box.value, serverURL: c.server_url, path: c.path, lang: 'zh-CN', dark: 'auto' })
        destroy = () => w?.destroy?.()
        break
      }
      case 'artalk': {
        loadCss(CDN.artalkCss)
        await loadScript(CDN.artalk)
        const a = g.Artalk!.init({
          el: box.value,
          server: c.server,
          site: c.site ?? undefined,
          pageKey: c.path,
          pageTitle: c.title,
          darkMode: 'auto',
        })
        destroy = () => a?.destroy?.()
        break
      }
      case 'giscus':
        scriptWidget(CDN.giscus, {
          'data-repo': c.repo,
          'data-repo-id': c.repo_id,
          'data-category': c.category,
          'data-category-id': c.category_id,
          'data-mapping': 'specific',
          'data-term': c.term,
          'data-reactions-enabled': '1',
          'data-input-position': 'top',
          'data-theme': 'preferred_color_scheme',
          'data-lang': 'zh-CN',
          'data-loading': 'lazy',
        })
        break
      case 'utterances':
        scriptWidget(CDN.utterances, {
          repo: c.repo,
          'issue-term': c.term,
          theme: dark() ? 'github-dark' : 'github-light',
        })
        break
    }
    state.value = 'ready'
  } catch (e) {
    console.warn('comment widget failed', e)
    state.value = 'error'
  }
}

onMounted(() => {
  observer = new IntersectionObserver(
    (entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        observer?.disconnect()
        void mount()
      }
    },
    { rootMargin: '200px' },
  )
  observer.observe(root.value!)
})

onBeforeUnmount(() => {
  observer?.disconnect()
  destroy?.()
})
</script>

<template>
  <div ref="root" class="embed">
    <p v-if="state === 'loading'" class="hint">正在加载原博客的评论区…</p>
    <p v-else-if="state === 'error'" class="hint">评论区加载失败，可以到原文评论。</p>
    <div :id="boxId" ref="box" />
  </div>
</template>

<style scoped>
.embed {
  margin-top: 12px;
  min-height: 60px;
}
.hint {
  margin: 0;
  color: var(--text-3);
  font-size: 13px;
}
</style>
