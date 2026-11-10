// Followed blogs live in a cookie on this device (no account). Recently opened
// posts are kept in localStorage; both only steer what "推荐" shows first.

import { readonly, ref } from 'vue'

const COOKIE = 'follows'
const MAX_FOLLOWS = 500
const MAX_AGE_DAYS = 400
const READ_KEY = 'recently-read'
const MAX_READ = 30

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
  const next = current.includes(id) ? current.filter((x) => x !== id) : [id, ...current].slice(0, MAX_FOLLOWS)
  writeCookie(next)
  followed.value = next
}

function readHistory(): number[] {
  try {
    const v = JSON.parse(localStorage.getItem(READ_KEY) ?? '[]')
    return Array.isArray(v) ? v.filter((n) => Number.isInteger(n)).slice(0, MAX_READ) : []
  } catch {
    return []
  }
}

export function recentlyRead(): number[] {
  return readHistory()
}

export function markRead(id: number) {
  try {
    const next = [id, ...readHistory().filter((x) => x !== id)].slice(0, MAX_READ)
    localStorage.setItem(READ_KEY, JSON.stringify(next))
  } catch {
    // Storage blocked (private mode): recommendations just stay generic.
  }
}
