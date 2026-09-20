/// <reference types="vite/client" />

/**
 * CSS Modules are typed loosely on purpose. Generating exact class names would
 * mean a build step whose only job is to catch typos that the styles
 * themselves make obvious the moment you look at the window.
 */
declare module '*.module.css' {
  const classes: Record<string, string>
  export default classes
}
