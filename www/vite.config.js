import { defineConfig } from 'vite';

// `base` stays relative so one build works wherever it is served from: at
// http://localhost:4173/ while it is being written, and at
// https://acltabontabon.com/scuttle/ once GitHub Pages has it. The apex
// belongs to the acltabontabon.github.io repository and cascades to every
// project site, so the /scuttle/ prefix comes from the repository name and
// is never written down anywhere here.
export default defineConfig({
  base: './',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    assetsInlineLimit: 2048,
  },
  server: { port: 5280 },
  preview: { port: 4173 },
});
