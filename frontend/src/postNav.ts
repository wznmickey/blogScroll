// Moving from an open post to the next or previous one (wheel, swipe, ↑ ↓),
// as in a short-video feed: in the order of the list it was opened from
// (the waterfall the reader clicked in), loading more of it near the end.

import { ref, shallowRef } from 'vue'
import type { PostCard } from './types'

export interface Sequence {
  /** The list as loaded so far, in its order. */
  items: () => readonly PostCard[]
  /** Loads the next page, if any. */
  more?: () => Promise<void>
}

export const sequence = shallowRef<Sequence | null>(null)

/** Which way the last move went, for the next post's entrance: 1 down, -1 up, 0 opened. */
export const direction = ref<1 | -1 | 0>(0)
