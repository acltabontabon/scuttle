import type { UpdateInfo } from '@/lib/types'
import { releasedOn } from './phrasing'
import { parseNotes } from './view'

import styles from './Updates.module.css'

/**
 * What the release says about itself.
 *
 * Text only. The notes come from the network, so they are shown as words —
 * never as markup — whatever they contain.
 */
export function ReleaseNotes({ info }: { info: UpdateInfo }) {
  const lines = parseNotes(info.notes)
  const date = releasedOn(info.date_unix)

  if (lines.length === 0) {
    return (
      <p className={styles.notesEmpty}>
        {date ? `Released ${date}. ` : ''}This release came with no notes.
      </p>
    )
  }

  return (
    <div className={styles.notes} tabIndex={0} role="region" aria-label={`Release notes for ${info.version}`}>
      {date && <p className={styles.notesDate}>Released {date}</p>}
      {lines.map((line, index) =>
        line.kind === 'heading' ? (
          <h4 key={index} className={styles.notesHeading}>
            {line.text}
          </h4>
        ) : line.kind === 'item' ? (
          <p key={index} className={styles.notesItem}>
            {line.text}
          </p>
        ) : (
          <p key={index} className={styles.notesText}>
            {line.text}
          </p>
        ),
      )}
    </div>
  )
}
