import type { Category, Confidence, RecommendedAction, Risk } from '@/lib/types'

/**
 * Presentation-only facts about the vocabulary the core sends.
 *
 * Titles come from Rust (`Category::title`) so every surface agrees; what
 * lives here is the interface's own business — the one-line description shown
 * under a pile, and the words used for verdicts.
 */

export const CATEGORY_BLURB: Record<Category, string> = {
  // Scuttle cannot see uninstall records, only what is left on disk, so this
  // says what it observed rather than asserting the software is gone.
  ghosts: 'Files that look left over from software Scuttle cannot find.',
  screenshots: 'Captures nobody has looked at in a while.',
  installers: 'Installers whose job is finished.',
  heavy_strays: 'Large things. Not junk — just large.',
  copies: 'The same bytes, more than once.',
  caches: 'Caches Scuttle can name the owner of.',
  developer_debris: 'Build output your tools can make again.',
  oddments: 'Things Scuttle noticed but cannot name.',
}

/**
 * The evocative names earn their keep, but a name like "Ghosts" is no use to
 * someone who has never seen this screen before — and no use at all to a
 * screen reader. This is the plain version, used for accessible names and
 * anywhere the illustration is doing the talking.
 */
export const CATEGORY_PLAIN: Record<Category, string> = {
  ghosts: 'possible leftovers from software that is no longer installed',
  screenshots: 'screenshots you have not looked at in a long time',
  installers: 'installers you have already run',
  heavy_strays: 'unusually large files',
  copies: 'files that exist more than once',
  caches: 'caches belonging to apps Scuttle could identify',
  developer_debris: 'build output your tools can regenerate',
  oddments: 'things Scuttle noticed but cannot identify',
}

/**
 * A short plain-language line for the categories whose names are evocative
 * but opaque on first meeting. Only these three: the rest say what they are.
 *
 * Each describes what the detector actually keys on, and none of them
 * promises the files are unwanted.
 */
export const CATEGORY_HINT: Partial<Record<Category, string>> = {
  ghosts: 'App files with no app to match',
  heavy_strays: 'Unusually large single files',
  oddments: 'Noticed, but Scuttle can’t identify',
}

/** Shown when a pile is empty but the category was searched. */
export const CATEGORY_NOTHING: Record<Category, string> = {
  ghosts: 'Everything here belongs to something installed.',
  screenshots: 'No forgotten captures.',
  installers: 'No installers hanging around.',
  heavy_strays: 'Nothing unusually large.',
  copies: 'No duplicates worth the disk space.',
  caches: 'Caches are all reasonable sizes.',
  developer_debris: 'No stale build output.',
  oddments: 'Nothing unexplained.',
}

export const CONFIDENCE_WORD: Record<Confidence, string> = {
  high: 'High',
  medium: 'Medium',
  low: 'Low',
}

export const RISK_WORD: Record<Risk, string> = {
  low: 'Low',
  moderate: 'Moderate',
  high: 'High',
  protected: 'Protected',
}

/**
 * What the risk level actually means for the person reading it. Written
 * plainly — this is safety copy, and safety copy is never funny.
 */
export const RISK_MEANING: Record<Risk, string> = {
  low: 'If this turns out to be wrong, it costs you a download or a rebuild.',
  moderate: 'Getting this back would take some effort.',
  high: 'This may be something you made. Scuttle will not suggest removing it.',
  protected: 'Scuttle will not act on this at all.',
}

export const ACTION_WORD: Record<RecommendedAction, string> = {
  quarantine: 'Worth quarantining',
  review: 'Worth a look',
  inspect_only: 'Just so you know',
}

export const ACTION_MEANING: Record<RecommendedAction, string> = {
  quarantine: 'Scuttle is confident, and being wrong would be cheap.',
  review: 'Scuttle has an opinion but would rather you decided.',
  inspect_only: 'Surfaced for awareness. Scuttle is not suggesting anything.',
}
