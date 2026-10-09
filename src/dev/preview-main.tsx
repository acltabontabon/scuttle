import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { Preview } from './Preview'
import { installPreviewApi } from './preview-api'

import '@/styles/base.css'

const root = document.getElementById('preview-root')
if (!root) throw new Error('nowhere to preview')

const restoreApi = installPreviewApi()
if (import.meta.hot) import.meta.hot.dispose(restoreApi)

createRoot(root).render(
  <StrictMode>
    <Preview />
  </StrictMode>,
)
