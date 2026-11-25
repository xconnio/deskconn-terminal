import { createApp } from 'vue'
import { createPinia } from 'pinia'
import 'bootstrap-icons/font/bootstrap-icons.css'
import App from './App.vue'
import { cliKeyPlugin } from './cliKey'

const pinia = createPinia()
pinia.use(cliKeyPlugin)

createApp(App).use(pinia).mount('#app')
