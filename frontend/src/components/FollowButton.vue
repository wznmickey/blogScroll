<script setup lang="ts">
import { computed } from 'vue'
import { follows, toggleFollow } from '../follow'

const props = withDefaults(defineProps<{ blogId: number; small?: boolean }>(), { small: false })

const on = computed(() => follows.value.includes(props.blogId))
</script>

<template>
  <button
    type="button"
    class="follow"
    :class="{ on, small }"
    :aria-pressed="on"
    @click.prevent.stop="toggleFollow(blogId)"
  >
    {{ on ? '已关注' : '关注' }}
  </button>
</template>

<style scoped>
.follow {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 32px;
  min-width: 68px;
  padding: 0 14px;
  border-radius: 999px;
  border: 1px solid var(--accent);
  background: var(--accent);
  color: #fff;
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  transition:
    background 0.15s,
    color 0.15s;
}
.follow.on {
  border-color: var(--line);
  background: transparent;
  color: var(--text-2);
}
.follow.small {
  height: 26px;
  min-width: 56px;
  padding: 0 10px;
  font-size: 12px;
}
</style>
