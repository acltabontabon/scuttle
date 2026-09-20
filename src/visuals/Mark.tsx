import { memo } from 'react'

/**
 * The wordmark's companion: Scuttle reduced to a silhouette that still reads
 * at 16 pixels. Same shell, same two eyes, no legs — at this size legs turn
 * into fuzz.
 */
function MarkImpl({ size = 16 }: { size?: number }) {
  return (
    <svg
      viewBox="0 0 24 20"
      width={size}
      height={size * (20 / 24)}
      aria-hidden="true"
      focusable="false"
      style={{ display: 'block', overflow: 'visible' }}
    >
      <path
        d="M12 3 C17 3 20 6.4 20 11 L20 14.6 L4 14.6 L4 11 C4 6.4 7 3 12 3 Z"
        fill="currentColor"
      />
      <circle cx="9.4" cy="10.2" r="1.9" fill="var(--honey)" />
      <circle cx="14.6" cy="10.2" r="1.9" fill="var(--honey)" />
      <path
        d="M4.6 14.6 L1.6 17.8 M8 14.6 L6 18.4 M19.4 14.6 L22.4 17.8 M16 14.6 L18 18.4"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        fill="none"
      />
    </svg>
  )
}

export const Mark = memo(MarkImpl)
