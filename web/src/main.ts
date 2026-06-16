import { createApp } from 'vue'
import App from './App.vue'
// S11-a: self-host the fonts the design already declares (styles.css / theme.ts
// referenced 'Inter' / 'JetBrains Mono' but nothing ever loaded them). @fontsource
// ships @font-face + woff2 with unicode-range subsetting; Vite emits them as local
// assets (no CDN, fits the local-backend topology). Only the weights actually used:
// Inter 400/600 (UI body + sidebar/status emphasis), JetBrains Mono 400/700 (code).
// CJK is uncovered by both → falls back to system-ui as before (intended); so we
// import the *latin* subset only (English UI + ASCII code), keeping the @font-face
// CSS lean — latin's unicode-range already covers our punctuation (·, ›, —).
import '@fontsource/inter/latin-400.css'
import '@fontsource/inter/latin-600.css'
import '@fontsource/jetbrains-mono/latin-400.css'
import '@fontsource/jetbrains-mono/latin-700.css'
import './styles.css'

createApp(App).mount('#app')
