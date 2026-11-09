<script setup lang="ts">
import { computed, ref } from 'vue'
import { hue, initial } from '../format'

const props = withDefaults(defineProps<{ name: string; icon?: string | null; size?: number }>(), {
  icon: null,
  size: 20,
})

const failed = ref(false)
const style = computed(() => ({
  width: `${props.size}px`,
  height: `${props.size}px`,
  fontSize: `${Math.round(props.size * 0.5)}px`,
  '--h': hue(props.name),
}))
</script>

<template>
  <span class="avatar" :style="style" aria-hidden="true">
    <img
      v-if="icon && !failed"
      :src="icon"
      alt=""
      loading="lazy"
      referrerpolicy="no-referrer"
      @error="failed = true"
    />
    <template v-else>{{ initial(name) }}</template>
  </span>
</template>

<style scoped>
.avatar {
  display: inline-flex;
  flex: none;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  overflow: hidden;
  background: hsl(var(--h) 55% 58%);
  color: #fff;
  font-weight: 600;
  line-height: 1;
  user-select: none;
}
.avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  background: var(--surface);
}
</style>
