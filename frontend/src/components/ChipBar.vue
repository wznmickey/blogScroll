<script setup lang="ts">
export interface Chip {
  /** `''` stands for "all". */
  value: string
  label: string
  count?: number
}

defineProps<{ chips: Chip[]; active: string; label: string }>()
const emit = defineEmits<{ select: [value: string] }>()
</script>

<template>
  <div class="chip-bar" role="toolbar" :aria-label="label">
    <button
      v-for="c in chips"
      :key="c.value"
      class="chip"
      :class="{ active: active === c.value }"
      :aria-pressed="active === c.value"
      @click="emit('select', c.value)"
    >
      {{ c.label }}<span v-if="c.count !== undefined" class="count">{{ c.count }}</span>
    </button>
    <slot />
  </div>
</template>

<style scoped>
.chip-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  overflow-x: auto;
  scrollbar-width: none;
  -webkit-overflow-scrolling: touch;
}
.chip-bar::-webkit-scrollbar {
  display: none;
}
.chip {
  flex: none;
  display: inline-flex;
  align-items: baseline;
  gap: 4px;
  height: 30px;
  padding: 0 14px;
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-2);
  font-size: 13px;
  line-height: 30px;
  white-space: nowrap;
  transition:
    background 0.15s,
    color 0.15s;
}
.chip:hover {
  color: var(--text);
}
.chip.active {
  background: var(--text);
  color: var(--bg);
  font-weight: 600;
}
.count {
  font-size: 11px;
  opacity: 0.6;
}
@media (max-width: 639px) {
  .chip {
    height: 28px;
    padding: 0 12px;
    line-height: 28px;
  }
}
</style>
