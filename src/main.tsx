import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import { ensureFullscreenViewport } from './viewport'
import { installOverlayScrollbars } from './overlayScrollbars'
import './app.css'

// Claim the whole window before React mounts: zero body margin (no white
// border), full height, no page scrollbar (Issue #4 follow-up).
ensureFullscreenViewport()

// Overlay scrollbars (#105 / story 11): the five scroll regions show a thin
// bar only while scrolling (fading after) or hovered — one shared module
// owns the styles and the flash behavior.
installOverlayScrollbars()

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
