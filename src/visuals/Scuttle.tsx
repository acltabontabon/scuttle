import { memo } from 'react'

import styles from './Scuttle.module.css'

/**
 * Scuttle itself.
 *
 * Deliberately not a pet: it has no needs, no name to set, no streak to keep,
 * and it never asks for anything. It is a small presence that reacts to work
 * the application is genuinely doing.
 *
 * Every mood is expressed in *pose* as well as motion, so the creature still
 * communicates with animation disabled. Removing this component entirely
 * should break nothing but the charm.
 */
export type Mood =
  | 'idle'
  | 'rummaging'
  | 'found'
  | 'asleep'
  | 'shrug'
  | 'disappointed'
  | 'straining'

const MOOD_LABEL: Record<Mood, string> = {
  idle: 'Scuttle, waiting',
  rummaging: 'Scuttle, rummaging',
  found: 'Scuttle, having found something',
  asleep: 'Scuttle, asleep',
  shrug: 'Scuttle, shrugging',
  disappointed: 'Scuttle, unimpressed',
  straining: 'Scuttle, struggling with something enormous',
}

interface ScuttleProps {
  mood?: Mood
  size?: number
  /** Flip horizontally, for when it wanders the other way. */
  facing?: 'left' | 'right'
  className?: string
}

/**
 * Three legs, mirrored into a pair each. They start under the shell — which
 * is drawn after them, so the join is hidden — and reach down and out.
 *
 * Each rotates about its own attachment point rather than about the creature's
 * centre, which is what makes the shuffle read as walking rather than as the
 * whole animal wobbling.
 */
const LEGS = [
  { className: 'legFront', d: 'M32 56 L10 62', origin: '32px 56px' },
  { className: 'legMiddle', d: 'M37 60 L17 75', origin: '37px 60px' },
  { className: 'legBack', d: 'M43 62 L31 83', origin: '43px 62px' },
] as const

function ScuttleImpl({ mood = 'idle', size = 96, facing = 'right', className }: ScuttleProps) {
  return (
    <svg
      viewBox="0 0 100 86"
      width={size}
      height={size * 0.86}
      role="img"
      aria-label={MOOD_LABEL[mood]}
      className={[styles.scuttle, styles[mood], className].filter(Boolean).join(' ')}
      style={facing === 'left' ? { transform: 'scaleX(-1)' } : undefined}
    >
      {/* Claws sit behind the shell so they read as reaching out from under it. */}
      <g className={`${styles.claw} ${styles.clawLeft}`}>
        <path d="M32 44 L20 35" />
        <circle className={styles.pincer} cx="18.5" cy="33.5" r="5" />
        <circle cx="21.5" cy="30" r="2.6" fill="var(--paper)" stroke="none" />
      </g>
      <g className={`${styles.claw} ${styles.clawRight}`}>
        <path d="M68 44 L80 35" />
        <circle className={styles.pincer} cx="81.5" cy="33.5" r="5" />
        <circle cx="78.5" cy="30" r="2.6" fill="var(--paper)" stroke="none" />
      </g>

      {/*
        The mirror lives on a wrapping group, not on the paths. Putting it on
        the paths would mean the CSS animation's `transform` replaced it, and
        the right-hand legs would snap across the creature the moment they
        started moving.
      */}
      {([1, -1] as const).map((side) => (
        <g key={side} transform={side === -1 ? 'translate(100, 0) scale(-1, 1)' : undefined}>
          {LEGS.map((leg) => (
            <path
              key={leg.className}
              className={`${styles.leg} ${styles[leg.className]}`}
              d={leg.d}
              style={{ transformOrigin: leg.origin }}
            />
          ))}
        </g>
      ))}

      <g className={styles.shell}>
        {/* A low pebble of a shell with a flat underside. */}
        <path d="M50 20 C67 20 79 31 79 46 L79 62 L21 62 L21 46 C21 31 33 20 50 20 Z" />
        <ellipse className={styles.eye} cx="42" cy="45" rx="5.4" ry="5.4" />
        <ellipse className={styles.eye} cx="58" cy="45" rx="5.4" ry="5.4" />
      </g>

      {mood === 'asleep' && (
        <>
          <text className={styles.snore} x="76" y="26">
            z
          </text>
          <text className={styles.snore} x="84" y="18">
            z
          </text>
        </>
      )}
    </svg>
  )
}

export const Scuttle = memo(ScuttleImpl)
