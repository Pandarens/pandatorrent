import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { PlayerWindow } from './PlayerWindow.tsx'

// The player runs in its own window, told apart by the query string the Rust
// side puts on its URL. It shares this bundle but none of the app chrome.
const isPlayer = new URLSearchParams(window.location.search).get('window') === 'player'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {isPlayer ? <PlayerWindow /> : <App />}
  </StrictMode>,
)
