import { createRouter, createWebHistory } from 'vue-router'
import HomeView from './views/HomeView.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', name: 'home', component: HomeView },
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
