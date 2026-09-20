/**
 * Which operating system the window is on.
 *
 * Used for two things only: leaving room for the macOS traffic lights, and
 * calling the tray by the name the platform uses for it. Anything that
 * actually depends on platform behaviour is decided in Rust, where the answer
 * is a compile-time fact rather than a guess at a user-agent string.
 */
export type Platform = 'macos' | 'windows' | 'other'

export function platform(): Platform {
  if (typeof navigator === 'undefined') return 'other'
  const agent = navigator.userAgent
  if (/Mac/i.test(agent)) return 'macos'
  if (/Win/i.test(agent)) return 'windows'
  return 'other'
}
