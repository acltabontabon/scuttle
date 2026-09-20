import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { Preview } from './Preview'

import '@/styles/base.css'

const root = document.getElementById('preview-root')
if (!root) throw new Error('nowhere to preview')

createRoot(root).render(
  <StrictMode>
    <Preview />
  </StrictMode>,
)
