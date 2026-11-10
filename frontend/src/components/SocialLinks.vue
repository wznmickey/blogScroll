<script setup lang="ts">
import { computed } from 'vue'
import { socialItems } from '../social'
import { socialIcon } from '../socialIcons'

// A blogger's accounts as a row of icons in their brand colours; the name
// shows on hover, and a further account on the same platform keeps its name
// beside the icon.
const props = defineProps<{ links: { kind: string; url: string }[] }>()

const items = computed(() => socialItems(props.links).map((s) => ({ ...s, icon: socialIcon(s.kind, s.url) })))
</script>

<template>
  <ul class="social">
    <li v-for="s in items" :key="s.url">
      <a
        :href="s.url"
        target="_blank"
        rel="noopener me"
        :title="s.label"
        :aria-label="s.label"
        :class="{ named: s.handle }"
        :style="{ '--brand': s.icon.color }"
      >
        <svg v-if="s.icon.path" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
          <path :d="s.icon.path" fill="currentColor" />
        </svg>
        <span v-else class="badge" aria-hidden="true">{{ s.icon.text }}</span>
        <span v-if="s.handle" class="handle">{{ s.handle }}</span>
      </a>
    </li>
  </ul>
</template>

<style scoped>
.social {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
}
a {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 30px;
  min-width: 30px;
  justify-content: center;
  padding: 0 7px;
  border-radius: 999px;
  border: 1px solid var(--line);
  background: var(--bg);
  color: var(--text-2);
  font-size: 12px;
  transition: border-color 0.15s;
}
svg,
.badge {
  flex: none;
  color: var(--brand);
}
a.named {
  padding: 0 10px 0 8px;
}
@media (hover: hover) {
  a:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--brand) 45%, transparent);
  }
}
@media (prefers-color-scheme: dark) {
  /* Black marks (X, Threads, Steam) would vanish on the dark page. */
  svg,
  .badge {
    color: color-mix(in srgb, var(--brand) 55%, var(--text));
  }
}
.badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 4px;
  /* A square in the link's colour, the letter cut out of it. */
  background: currentColor;
  -webkit-text-fill-color: var(--bg);
  font-size: 10px;
  font-weight: 700;
  line-height: 1;
}
.handle {
  max-width: 9em;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
