import { createRouter, createWebHashHistory, createWebHistory } from 'vue-router'
import { isStatic } from './api'
import HomeView from './views/HomeView.vue'

export const router = createRouter({
  // Static hosts cannot rewrite deep links to index.html, so use #/ URLs there.
  history: isStatic ? createWebHashHistory(import.meta.env.BASE_URL) : createWebHistory(import.meta.env.BASE_URL),
  routes: [
    { path: '/', name: 'home', component: HomeView },
    { path: '/blogs', name: 'blogs', component: () => import('./views/BlogsView.vue') },
    { path: '/blog/:id(\\d+)', name: 'blog', component: () => import('./views/BlogView.vue') },
    { path: '/about', name: 'about', component: () => import('./views/AboutView.vue') },
    { path: '/:pathMatch(.*)*', redirect: '/' },
  ],
  scrollBehavior(to, from, saved) {
    // Opening/closing a post (?p=) or switching tabs must not jump the feed.
    if (to.path === from.path) return false
    return saved ?? { top: 0 }
  },
})
