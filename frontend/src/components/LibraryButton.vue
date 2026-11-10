<script setup lang="ts">
// 喜欢 (heart) or 收藏 (star) for a post; kept in this browser (library.ts).
import { computed } from 'vue'
import { has, toggle } from '../library'
import type { PostCard } from '../types'

const props = withDefaults(defineProps<{ post: PostCard; list: 'likes' | 'saves'; label?: boolean; size?: number }>(), {
  label: false,
  size: 20,
})

const on = computed(() => has(props.list, props.post.id))
const name = computed(() => (props.list === 'likes' ? '喜欢' : '收藏'))
</script>

<template>
  <button
    type="button"
    class="lib"
    :class="[list, { on }]"
    :aria-pressed="on"
    :aria-label="name"
    :title="on ? `取消${name}` : name"
    @click.prevent.stop="toggle(list, post)"
  >
    <svg
      v-if="list === 'likes'"
      viewBox="0 0 24 24"
      :width="size"
      :height="size"
      :fill="on ? 'currentColor' : 'none'"
      stroke="currentColor"
      stroke-width="2"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1-1.1a5.5 5.5 0 0 0-7.8 7.8l1 1.1L12 21l7.8-7.5 1-1.1a5.5 5.5 0 0 0 0-7.8z" />
    </svg>
    <svg
      v-else
      viewBox="0 0 24 24"
      :width="size"
      :height="size"
      :fill="on ? 'currentColor' : 'none'"
      stroke="currentColor"
      stroke-width="2"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3 6.4 20.2l1.1-6.2L3 9.6l6.2-.9z" />
    </svg>
    <span v-if="label">{{ on ? `已${name}` : name }}</span>
  </button>
</template>

<style scoped>
.lib {
  display: inline-flex;
  flex: none;
  align-items: center;
  gap: 4px;
  color: var(--text-2);
  font-size: 13px;
  transition:
    color 0.15s,
    transform 0.15s;
}
.lib:hover {
  color: var(--text);
}
.lib:active {
  transform: scale(0.9);
}
.likes.on {
  color: var(--accent);
}
.saves.on {
  color: #f5a623;
}
</style>
