<script setup lang="ts">
import { isStatic, meta } from '../api'
import { fullDate } from '../format'
</script>

<template>
  <div class="container">
    <article class="about">
      <h1>关于{{ meta?.site_name ?? '' }}</h1>
      <p>
        这里把独立博客的最新文章做成瀑布流，方便随手翻阅、发现新的博主。点开任意卡片会看到摘要，阅读全文请到原博客。
      </p>

      <h2>数据来源</h2>
      <ul>
        <li v-for="d in meta?.directories ?? []" :key="d.id">
          <a :href="d.homepage" target="_blank" rel="noopener">{{ d.name }}</a> 的公开成员列表
        </li>
      </ul>
      <p v-if="meta" class="stats">
        目前收录 {{ meta.stats.blogs }} 个博客，其中 {{ meta.stats.blogs_with_feed }} 个已成功读取 RSS，共
        {{ meta.stats.posts }} 篇文章。
        <template v-if="meta.stats.last_directory_sync">成员列表更新于 {{ fullDate(meta.stats.last_directory_sync) }}。</template>
        <template v-if="isStatic">本页是定时生成的静态版本，数据生成于 {{ fullDate(meta.generated_at) }}。</template>
      </p>

      <h2>我们如何抓取</h2>
      <ul>
        <li>只读取各博客公开的 RSS / Atom 订阅源，不抓取文章页面。</li>
        <li>默认只保存标题、约 200 字摘要和头图链接；图片直接从原站加载，本站不存储。</li>
        <li>每个订阅源通常几小时才请求一次，长期不更新的会逐渐降低频率；使用条件请求（ETag / Last-Modified），并遵守你在 Cache-Control、Expires 和 RSS ttl 里声明的缓存时间。</li>
        <li>同一网站的两次请求之间至少间隔数秒；遇到错误、429 或 Retry-After 会自动退避。</li>
      </ul>

      <h2>给博主</h2>
      <ul>
        <li>不希望被收录？联系站长即可，你的站点会被排除，已存的内容也会删除。</li>
        <li>希望在这里展示全文？在获得你的明确授权后，我们才会保存全文。</li>
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
