// First: a pairing link's code (#sync=) leaves the address before the router reads it.
import './sync'
import { createApp } from 'vue'
import App from './App.vue'
import { router } from './router'
import './style.css'

createApp(App).use(router).mount('#app')
