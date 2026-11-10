// Sideways scrolling of a chip row with a mouse: the wheel scrolls it
// horizontally, and it can be dragged. Touch and trackpads already scroll it
// natively. A drag that moved does not count as a click on the chip under it.

import { onBeforeUnmount, ref, watch, type Ref } from 'vue'

/** Pixels the pointer must move before a press becomes a drag. */
const DRAG_THRESHOLD = 5

export function useDragScroll(el: Ref<HTMLElement | undefined>) {
  /** Whether there is more to see on each side (for arrows and fades). */
  const canLeft = ref(false)
  const canRight = ref(false)
  const cleanups: (() => void)[] = []

  function update() {
    const n = el.value
    if (!n) return
    canLeft.value = n.scrollLeft > 1
    canRight.value = n.scrollLeft + n.clientWidth < n.scrollWidth - 1
  }

  /** Scrolls by most of a screen's width; `dir` is -1 (left) or 1 (right). */
  function scrollByPage(dir: -1 | 1) {
    const n = el.value
    if (n) n.scrollBy({ left: dir * n.clientWidth * 0.7, behavior: 'smooth' })
  }

  // The element may come and go (v-if); listeners follow it.
  function attach(n: HTMLElement | undefined) {
    cleanups.splice(0).forEach((f) => f())
    if (!n) return
    const on = <K extends keyof HTMLElementEventMap>(
      type: K,
      fn: (e: HTMLElementEventMap[K]) => void,
      opts?: AddEventListenerOptions,
    ) => {
      n.addEventListener(type, fn, opts)
      cleanups.push(() => n.removeEventListener(type, fn, opts))
    }

    on(
      'wheel',
      (e) => {
        // Horizontal gestures and pinch-zoom are left alone.
        if (e.ctrlKey || Math.abs(e.deltaX) >= Math.abs(e.deltaY)) return
        const max = n.scrollWidth - n.clientWidth
        if (max <= 0) return
        const dy = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaMode === 2 ? e.deltaY * n.clientWidth : e.deltaY
        const next = Math.min(max, Math.max(0, n.scrollLeft + dy))
        // At either end the wheel scrolls the page as usual.
        if (next === n.scrollLeft) return
        e.preventDefault()
        n.scrollLeft = next
      },
      { passive: false },
    )

    let pressed = false
    let dragged = false
    let startX = 0
    let startLeft = 0
    on('pointerdown', (e) => {
      if (e.pointerType !== 'mouse' || e.button !== 0) return
      pressed = true
      dragged = false
      startX = e.clientX
      startLeft = n.scrollLeft
    })
    on('pointermove', (e) => {
      if (!pressed) return
      const dx = e.clientX - startX
      if (!dragged) {
        if (Math.abs(dx) < DRAG_THRESHOLD) return
        dragged = true
        n.setPointerCapture(e.pointerId)
        n.classList.add('dragging')
      }
      n.scrollLeft = startLeft - dx
    })
    const release = () => {
      pressed = false
      n.classList.remove('dragging')
    }
    on('pointerup', release)
    on('pointercancel', release)
    on(
      'click',
      (e) => {
        if (!dragged) return
        e.preventDefault()
        e.stopPropagation()
        dragged = false
      },
      { capture: true },
    )
    on('dragstart', (e) => e.preventDefault())
    on('scroll', update, { passive: true })

    // Chips come and go (channels added, categories loaded), and widths change.
    const resize = new ResizeObserver(update)
    resize.observe(n)
    const mutation = new MutationObserver(update)
    mutation.observe(n, { childList: true, subtree: true, characterData: true })
    cleanups.push(() => resize.disconnect(), () => mutation.disconnect())
    update()
  }

  watch(el, attach, { immediate: true, flush: 'post' })
  onBeforeUnmount(() => attach(undefined))

  return { canLeft, canRight, scrollByPage }
}
