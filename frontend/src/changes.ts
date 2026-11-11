// What the stores tell sync.ts: that something changed (so it is written out
// soon), and what the reader removed (so other devices remove it too, rather
// than bringing it back). Only the reader's own actions are reported, not
// lists getting shorter at their limit or a sync putting things in place.

export type Change =
  /** `soft`: frequent and minor (刷到过, history, positions): written less often. */
  | { kind: 'changed'; soft?: boolean }
  | { kind: 'removed'; collection: string; keys: (string | number)[] }
  | { kind: 'followed'; id: number }

const listeners: ((c: Change) => void)[] = []

export function onChange(listener: (c: Change) => void) {
  listeners.push(listener)
}

export function report(c: Change) {
  for (const l of listeners) l(c)
}
