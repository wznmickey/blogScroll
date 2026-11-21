// First: what goes wrong from here on is noted for 设置 → 诊断信息.
import { watchComponents } from './diagnostics'
// Then: a pairing link's code (#sync=) leaves the address before the router reads it.
import './sync'
import { createApp } from 'vue'
import App from './App.vue'
import { router } from './router'
import './style.css'

const app = createApp(App)
watchComponents(app)
app.use(router).mount('#app')
