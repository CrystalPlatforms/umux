import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import { ensureFullscreenViewport } from './viewport'
import './app.css'

// Claim the whole window before React mounts: zero body margin (no white
// border), full height, no page scrollbar (Issue #4 follow-up).
ensureFullscreenViewport()

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
