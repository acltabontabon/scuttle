import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { App } from '@/app/App'
import { StoreProvider } from '@/app/store'

import '@/styles/base.css'

const root = document.getElementById('scuttle-root')
if (!root) throw new Error('Scuttle has nowhere to live')

createRoot(root).render(
  <StrictMode>
    <StoreProvider>
      <App />
    </StoreProvider>
  </StrictMode>,
)
