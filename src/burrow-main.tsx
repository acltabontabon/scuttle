import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { Burrow } from '@/features/burrow/Burrow'

import '@/styles/base.css'

const root = document.getElementById('burrow-root')
if (!root) throw new Error('The burrow has nowhere to live')

createRoot(root).render(
  <StrictMode>
    <Burrow />
  </StrictMode>,
)
