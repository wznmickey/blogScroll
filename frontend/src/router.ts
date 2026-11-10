import { createRouter, createWebHistory } from 'vue-router'
import HomeView from './views/HomeView.vue'

// Links from before real paths (static hosting used #/ URLs): #/blog/12 → blog/12.
// Before the router is made: it takes the address as it is when created.
if (location.hash.startsWith('#/')) {
  history.replaceState(null, '', import.meta.env.BASE_URL + location.hash.slice(2))
}

export const router = createRouter({
  // Real paths everywhere. On static hosting the export writes a page for
  // each route search engines should see (blog/12.html, ...) and 404.html
  // (the app) catches the rest, e.g. /search.
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    { path: '/', name: 'home', component: HomeView },
    { path: '/blogs', name: 'blogs', component: () => import('./views/BlogsView.vue') },
    { path: '/blog/:id(\\d+)', name: 'blog', component: () => import('./views/BlogView.vue') },
    { path: '/search', name: 'search', component: () => import('./views/SearchView.vue') },
    { path: '/me', name: 'me', component: () => import('./views/MeView.vue') },
    { path: '/about', name: 'about', component: () => import('./views/AboutView.vue') },
    { path: '/:pathMatch(.*)*', redirect: '/' },
  ],
  scrollBehavior(to, from, saved) {
    // Opening/closing a post (?p=) or switching tabs must not jump the feed.
    if (to.path === from.path) return false
    return saved ?? { top: 0 }
  },
})
