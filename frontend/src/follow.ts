// Followed blogs live in a cookie on this device (no account), so they work
// the same with the server and on static hosting.

import { readonly, ref } from 'vue'
import { report } from './changes'

const COOKIE = 'follows'
const MAX_FOLLOWS = 500
const MAX_AGE_DAYS = 400

function readCookie(): number[] {
  const raw = document.cookie
    .split(';')
    .map((c) => c.trim())
    .find((c) => c.startsWith(`${COOKIE}=`))
    ?.slice(COOKIE.length + 1)
  // "." separated: commas are not allowed in cookie values.
  return (raw ?? '')
    .split('.')
    .map(Number)
    .filter((n) => Number.isInteger(n) && n > 0)
}

function writeCookie(ids: number[]) {
  const path = import.meta.env.BASE_URL || '/'
  const secure = location.protocol === 'https:' ? '; secure' : ''
  document.cookie = ids.length
    ? `${COOKIE}=${ids.join('.')}; max-age=${MAX_AGE_DAYS * 86400}; path=${path}; samesite=lax${secure}`
    : `${COOKIE}=; max-age=0; path=${path}`
}

const followed = ref<number[]>(readCookie())
export const follows = readonly(followed)

// Another tab may have changed the cookie.
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible') followed.value = readCookie()
})

export function isFollowing(id: number): boolean {
  return followed.value.includes(id)
}

export function toggleFollow(id: number) {
  const current = readCookie()
  const was = current.includes(id)
  const next = was ? current.filter((x) => x !== id) : [id, ...current].slice(0, MAX_FOLLOWS)
  writeCookie(next)
  followed.value = next
  report(was ? { kind: 'removed', collection: 'follows', keys: [id] } : { kind: 'followed', id })
  report({ kind: 'changed' })
}

/** Restoring a backup: the given blogs, or those added to the current ones. */
export function restoreFollows(ids: number[], mode: 'merge' | 'replace') {
  const clean = ids.filter((n) => Number.isInteger(n) && n > 0)
  const next = [...new Set(mode === 'merge' ? [...readCookie(), ...clean] : clean)].slice(0, MAX_FOLLOWS)
  writeCookie(next)
  followed.value = next
}
