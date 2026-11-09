<script setup lang="ts">
import { computed } from 'vue'
import { hue } from '../format'
import type { PostCard } from '../types'

// Posts without a picture get a typographic cover, like text-only notes on XHS.
const props = defineProps<{ post: PostCard; ratio: number; large?: boolean }>()

const text = computed(() => {
  const ex = props.post.excerpt.trim()
  return ex.length >= 20 ? ex : props.post.title
})
const style = computed(() => ({
  aspectRatio: props.large ? undefined : `1 / ${props.ratio}`,
  '--h': hue(props.post.blog.name),
}))
</script>

<template>
  <div class="text-cover" :class="{ large }" :style="style">
    <span class="quote" aria-hidden="true">“</span>
    <p>{{ text }}</p>
  </div>
</template>

<style scoped>
.text-cover {
  position: relative;
  display: flex;
  align-items: center;
  overflow: hidden;
  padding: 14% 12% 12%;
  border-radius: var(--radius-cover);
  background: linear-gradient(150deg, hsl(var(--h) 75% 94%), hsl(calc(var(--h) + 40) 70% 86%));
  color: hsl(var(--h) 35% 24%);
}
@media (prefers-color-scheme: dark) {
  .text-cover {
    background: linear-gradient(150deg, hsl(var(--h) 28% 22%), hsl(calc(var(--h) + 40) 30% 15%));
    color: hsl(var(--h) 60% 88%);
  }
}
.quote {
  position: absolute;
  top: 2%;
  left: 7%;
  font-family: Georgia, serif;
  font-size: 56px;
  line-height: 1;
  opacity: 0.25;
}
p {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  line-height: 1.65;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 6;
  line-clamp: 6;
  overflow: hidden;
  word-break: break-word;
}
.large {
  min-height: 100%;
  padding: 64px 48px;
}
.large .quote {
  top: 64px;
  left: 40px;
  font-size: 72px;
}
.large p {
  font-size: 22px;
  -webkit-line-clamp: 12;
  line-clamp: 12;
}
@media (max-width: 639px) {
  p {
    font-size: 14px;
    -webkit-line-clamp: 5;
    line-clamp: 5;
  }
}
</style>
