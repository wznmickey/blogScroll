<script setup lang="ts">
import { computed } from 'vue'
import { isStatic, meta } from '../api'
import { fullDate, hostOf } from '../format'
import { openInReader } from '../reader'

const maintainer = computed(() => meta.value?.maintainer ?? null)

function openBlog(e: MouseEvent) {
  const m = maintainer.value
  if (!m?.url) return
  const blog = { id: m.blog?.id ?? 0, name: m.blog?.name ?? m.name, icon: null }
  if (openInReader({ url: m.url, title: blog.name, blog }, e)) e.preventDefault()
}
</script>

<template>
  <div class="container">
    <article class="about">
      <h1>关于{{ meta?.site_name ?? '' }}</h1>
      <p>
        这里把独立博客的最新文章做成瀑布流，方便随手翻阅、发现新的博主。点开任意卡片会看到摘要，阅读全文请到原博客。
      </p>

      <template v-if="maintainer">
        <h2>维护者</h2>
        <p class="maintainer">
          本站由 <b>{{ maintainer.name }}</b> 维护。
          <template v-if="maintainer.url">
            博客：<a :href="maintainer.url" target="_blank" rel="noopener me" @click="openBlog">{{
              maintainer.blog?.name ?? hostOf(maintainer.url)
            }}</a>
            <template v-if="maintainer.blog">
              （<RouterLink :to="`/blog/${maintainer.blog.id}`">在本站浏览</RouterLink>）
            </template>
          </template>
        </p>
      </template>

      <h2>数据来源</h2>
      <ul>
        <li v-for="d in meta?.directories ?? []" :key="d.id">
          <template v-if="d.homepage">
            <a :href="d.homepage" target="_blank" rel="noopener">{{ d.name }}</a> 的公开成员列表
          </template>
          <template v-else>站长另外订阅的 RSS（{{ d.name }}）</template>
        </li>
      </ul>
      <p v-if="meta" class="stats">
        目前收录 {{ meta.stats.blogs }} 个博客，其中 {{ meta.stats.blogs_with_feed }} 个已成功读取 RSS，共
        {{ meta.stats.posts }} 篇文章。
        <template v-if="meta.stats.last_directory_sync">成员列表更新于 {{ fullDate(meta.stats.last_directory_sync) }}。</template>
        <template v-if="isStatic">本页是定时生成的静态版本，数据生成于 {{ fullDate(meta.generated_at) }}。</template>
      </p>

      <h2>你的数据</h2>
      <ul>
        <li>
          不需要账号。关注、喜欢、收藏、浏览历史、刷到过的文章、各频道看到的位置和频道设置只保存在你这台设备的浏览器里。
        </li>
        <li>「推荐」会参考这些记录：请求时只带上文章和博客的编号，服务器不会写进数据库，也不缓存这类请求；不想要个性化可以看「发现」。</li>
        <li>换设备或清理浏览器前，可以在<RouterLink to="/me?tab=backup">「我的 → 备份与恢复」</RouterLink>下载备份文件，之后再恢复。</li>
      </ul>

      <h2>我们如何抓取</h2>
      <ul>
        <li>只读取各博客公开的 RSS / Atom 订阅源，不抓取文章页面。</li>
        <li>默认只保存标题、约 200 字摘要和头图链接；图片直接从原站加载，本站不存储。</li>
        <li>每个订阅源通常几小时才请求一次，长期不更新的会逐渐降低频率；使用条件请求（ETag / Last-Modified），并遵守你在 Cache-Control、Expires 和 RSS ttl 里声明的缓存时间。</li>
        <li>同一网站的两次请求之间至少间隔数秒；遇到错误、429 或 Retry-After 会自动退避。</li>
        <li>每月读一次博客首页和关于页，用于展示简介、头像和社交链接，并识别评论系统。</li>
        <li>评论数优先取自 RSS；否则每天最多向博客的评论服务发一次批量计数请求。评论服务拒绝访问时，30 天内不再请求。</li>
      </ul>

      <h2>给博主</h2>
      <ul>
        <li>不希望被收录？联系站长即可，你的站点会被排除，已存的内容也会删除。</li>
        <li>希望在这里展示全文？在获得你的明确授权后，我们才会保存全文。</li>
        <li>希望读者在这里直接评论？授权后，文章详情会加载你博客自己的评论组件，评论仍保存在你的评论服务里（需要把本站域名加入评论服务的白名单）。</li>
      </ul>
    </article>
  </div>
</template>

<style scoped>
.about {
  max-width: 720px;
  margin: 24px auto 64px;
  padding: 0 8px;
}
h1 {
  font-size: 26px;
  margin: 0 0 12px;
}
h2 {
  margin: 28px 0 8px;
  font-size: 18px;
}
p,
li {
  color: var(--text-2);
}
ul {
  padding-left: 1.2em;
}
li {
  margin: 4px 0;
}
a {
  color: var(--link);
}
.stats {
  padding: 12px 14px;
  border-radius: 12px;
  background: var(--surface);
  font-size: 14px;
}
</style>
