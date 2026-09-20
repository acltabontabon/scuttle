/**
 * Fixtures for the design workbench.
 *
 * These exist so the interface can be looked at — every pile, every verdict,
 * every empty state — without waiting for a real rummage to turn up the right
 * combination. They are reachable only from `preview.html`, which is not part
 * of the shipping application: nothing in `src/app` or `src/features` imports
 * this file, and the production bundle does not contain it.
 *
 * The shapes are copied from real scan output rather than invented, so the
 * workbench stays honest about what the core actually produces.
 */

import type {
  Candidate,
  Evidence,
  Findings,
  QuarantineView,
  Settings,
  SpaceOverview,
} from '@/lib/types'

const DAY = 86400
const now = Math.floor(Date.now() / 1000)

function reason(summary: string, weight: number, riskFloor: Evidence['risk_floor'] = null): Evidence {
  return { kind: { kind: 'FIXTURE' }, summary, weight, negative: weight < 0, risk_floor: riskFloor }
}

function candidate(partial: Partial<Candidate> & Pick<Candidate, 'id' | 'display_name'>): Candidate {
  const group = partial.group ?? []
  return {
    // Mirrors `model::group_footprint` on the Rust side: a group is worth the
    // sum of its members, anything else is worth its own size.
    group_bytes:
      group.length > 1
        ? group.reduce((total, member) => total + member.size, 0)
        : (partial.size ?? 1024 ** 3),
    detector: 'ghosts',
    category: 'ghosts',
    target_kind: 'directory',
    path: `/Users/rummager/Library/Application Support/${partial.display_name}`,
    associated_app: null,
    size: 1024 ** 3,
    confidence: 'high',
    risk: 'low',
    recommended_action: 'quarantine',
    evidence: [],
    remark: null,
    modified_unix: now - 142 * DAY,
    accessed_unix: now - 142 * DAY,
    created_unix: now - 800 * DAY,
    group: [],
    fingerprint: { size: 0, modified_unix: null, is_dir: true, child_count: 4 },
    ...partial,
  }
}

const GHOSTS: Candidate[] = [
  candidate({
    id: 'g1',
    display_name: 'Cyberpunk 2077',
    associated_app: 'Cyberpunk 2077',
    path: '/Users/rummager/Library/Application Support/Steam/steamapps/common/Cyberpunk 2077',
    size: 6.4 * 1024 ** 3,
    remark: 'Steam has no installation of Cyberpunk 2077.\n\n6.40 GB stayed behind, untouched for 142 days.',
    evidence: [
      reason('Steam has no installation of Cyberpunk 2077', 35),
      reason('Untouched for 142 days', 7),
      reason('Contents look generated — 91% of it is cache and log files', 25),
      reason('Nothing appears to be using it', 15),
    ],
  }),
  candidate({
    id: 'g2',
    display_name: 'com.sublimetext.4',
    size: 512 * 1024 ** 2,
    confidence: 'medium',
    recommended_action: 'review',
    risk: 'moderate',
    remark: 'The app is gone.\n\nThis isn’t.\n\n512 MB, last touched 310 days ago.',
    evidence: [
      reason('com.sublimetext.4 is not installed', 45),
      reason('Named for the application id com.sublimetext.4', 25),
      reason('Untouched for 310 days', 15),
      reason('2 files in here look like your own work', -70, 'high'),
    ],
  }),
  candidate({
    id: 'g3',
    display_name: 'Old Studio RPG',
    associated_app: 'Old Studio RPG',
    size: 2.1 * 1024 ** 3,
    confidence: 'low',
    risk: 'high',
    recommended_action: 'inspect_only',
    remark:
      'Old Studio RPG is gone, but this is not only leftovers.\n\nThere is something in here that looks like saved progress. Scuttle will not suggest removing it.',
    evidence: [
      reason('Old Studio RPG is not installed', 45),
      reason('Untouched for 400 days', 20),
      reason('Looks like save data — 9 files in here look like saved progress', -80, 'high'),
    ],
  }),
]

const SCREENSHOTS: Candidate[] = [
  candidate({
    id: 's1',
    detector: 'screenshots',
    category: 'screenshots',
    target_kind: 'file',
    display_name: 'Screenshot 2024-03-08 at 16.41.22.png',
    path: '/Users/rummager/Desktop/Screenshot 2024-03-08 at 16.41.22.png',
    size: 88 * 1024 ** 2,
    confidence: 'high',
    risk: 'moderate',
    recommended_action: 'review',
    remark: '18 screenshots.\n\nAll the same thing.\n\nRough afternoon?',
    evidence: [
      reason('18 images here look nearly identical', 30, 'moderate'),
      reason('Named like a screenshot (screenshot)', 25, 'moderate'),
      reason('3024x1964 — matches a display, not a camera', 10),
      reason('Untouched for 208 days', 10),
    ],
    group: Array.from({ length: 18 }, (_, i) => ({
      path: `/Users/rummager/Desktop/Screenshot 2024-03-08 at 16.4${i % 10}.22.png`,
      size: 5 * 1024 ** 2,
      modified_unix: now - (208 - i) * DAY,
      suggested_keep: i === 0,
    })),
  }),
]

const INSTALLERS: Candidate[] = [
  candidate({
    id: 'i1',
    detector: 'installers',
    category: 'installers',
    target_kind: 'file',
    display_name: 'Google Chrome.dmg',
    path: '/Users/rummager/Downloads/Google Chrome.dmg',
    associated_app: 'Google Chrome',
    size: 212 * 1024 ** 2,
    remark: 'Google Chrome is already installed.\n\nThis arrived 184 days ago.',
    evidence: [
      reason('A .dmg installer package', 30),
      reason('Google Chrome is already installed', 45),
      reason('Untouched for 184 days', 9),
    ],
  }),
  candidate({
    id: 'i2',
    detector: 'installers',
    category: 'installers',
    target_kind: 'file',
    display_name: 'Chrome (4).dmg',
    path: '/Users/rummager/Downloads/Chrome (4).dmg',
    associated_app: 'Google Chrome',
    size: 208 * 1024 ** 2,
    remark: 'You downloaded this 5 times.\n\nScuttle counted twice to be sure.',
    evidence: [
      reason('A .dmg installer package', 30),
      reason('Google Chrome is already installed', 45),
      reason('A newer copy exists: Google Chrome.dmg', 35),
    ],
  }),
]

const HEAVY: Candidate[] = [
  candidate({
    id: 'h1',
    detector: 'heavy',
    category: 'heavy_strays',
    target_kind: 'file',
    display_name: 'android-studio-backup.zip',
    path: '/Users/rummager/Downloads/android-studio-backup.zip',
    size: 18.7 * 1024 ** 3,
    confidence: 'medium',
    risk: 'high',
    recommended_action: 'inspect_only',
    remark:
      '18.7 GB, last touched 334 days ago.\n\nScuttle isn’t calling this junk. But that’s a lot of storage.',
    evidence: [reason('18.7 GB on disk', 10), reason('Untouched for 334 days', 16)],
  }),
]

const COPIES: Candidate[] = [
  candidate({
    id: 'c1',
    detector: 'duplicates',
    category: 'copies',
    target_kind: 'file',
    display_name: 'quarterly-report.pdf',
    path: '/Users/rummager/Downloads/archive/quarterly-report.pdf',
    size: 42 * 1024 ** 2,
    confidence: 'high',
    risk: 'moderate',
    recommended_action: 'review',
    remark: 'There are 3 of these.\n\nSpread across 3 folders. 84 MB of it is repetition.',
    evidence: [reason('2 other byte-identical copies exist', 50), reason('Untouched for 190 days', 9)],
    group: [
      { path: '/Users/rummager/Downloads/quarterly-report.pdf', size: 42 * 1024 ** 2, modified_unix: now - 20 * DAY, suggested_keep: true },
      { path: '/Users/rummager/Downloads/archive/quarterly-report.pdf', size: 42 * 1024 ** 2, modified_unix: now - 190 * DAY, suggested_keep: false },
      { path: '/Users/rummager/Desktop/quarterly-report (1).pdf', size: 42 * 1024 ** 2, modified_unix: now - 120 * DAY, suggested_keep: false },
    ],
  }),
]

const CACHES: Candidate[] = [
  candidate({
    id: 'k1',
    detector: 'caches',
    category: 'caches',
    display_name: 'Slack cache',
    associated_app: 'Slack',
    path: '/Users/rummager/Library/Application Support/Slack/Cache',
    size: 1.4 * 1024 ** 3,
    remark: '1.40 GB of Slack cache.\n\nRebuilt automatically, but the app should be closed first.',
    evidence: [
      reason('A known cache location for Slack', 55),
      reason('Slack rebuilds this when it needs it', 30),
      reason('Nothing appears to be using it', 15),
    ],
  }),
]

const ODDMENTS: Candidate[] = [
  candidate({
    id: 'o1',
    detector: 'ghosts',
    category: 'oddments',
    display_name: 'vendor-dump-2019',
    path: '/Users/rummager/Library/Application Support/vendor-dump-2019',
    size: 900 * 1024 ** 2,
    confidence: 'low',
    risk: 'moderate',
    recommended_action: 'inspect_only',
    remark: '900 MB, untouched for 612 days.\n\nScuttle has no idea what this is.\n\nYou decide.',
    evidence: [
      reason('Scuttle is not sure what this is — nothing installed claims this folder', 0, 'moderate'),
      reason('Untouched for 612 days', 25),
      reason('900 MB on disk', 10),
    ],
  }),
]

function pile(items: Candidate[]) {
  return {
    category: items[0]!.category,
    title: {
      ghosts: 'Ghosts',
      screenshots: 'Screenshots',
      installers: 'Installers',
      heavy_strays: 'Heavy strays',
      copies: 'Copies',
      caches: 'Caches',
      developer_debris: 'Developer debris',
      oddments: 'Oddments',
    }[items[0]!.category],
    bytes: items.reduce((total, item) => total + item.size, 0),
    count: items.length,
    actionable: items.filter((item) => item.recommended_action !== 'inspect_only').length,
    confident_count: items.filter((item) => item.recommended_action === 'quarantine').length,
    confident_bytes: items
      .filter((item) => item.recommended_action === 'quarantine')
      .reduce((total, item) => total + item.size, 0),
    items,
  }
}

export const FINDINGS: Findings = {
  scan_id: 'preview',
  finished_unix: now - 60,
  piles: [
    pile(GHOSTS),
    pile(HEAVY),
    pile(SCREENSHOTS),
    pile(INSTALLERS),
    pile(CACHES),
    pile(COPIES),
    pile(ODDMENTS),
  ],
  total_bytes: 29.5 * 1024 ** 3,
  reclaimable_bytes: 8.6 * 1024 ** 3,
  files_seen: 184_233,
  hiccups: { permission_denied: 12, vanished: 3, unreadable: 0, loops_avoided: 0 },
  has_rummaged: true,
}

export const EMPTY_FINDINGS: Findings = {
  ...FINDINGS,
  piles: [],
  total_bytes: 0,
  reclaimable_bytes: 0,
}

export const DRAWER: QuarantineView = {
  retention_days: 14,
  held_bytes: 6.9 * 1024 ** 3,
  items: [
    {
      id: 'q1',
      finding_id: 'g1',
      original_path: '/Users/rummager/Library/Application Support/Steam/steamapps/common/Cyberpunk 2077',
      stored_path: '/Users/rummager/Library/Application Support/Scuttle/Quarantine/q1/Cyberpunk 2077',
      display_name: 'Cyberpunk 2077',
      category: 'ghosts',
      size: 6.4 * 1024 ** 3,
      content_hash: null,
      evidence: [],
      quarantined_unix: now - 2 * DAY,
      expires_unix: now + 12 * DAY,
      status: 'held',
      resolved_unix: null,
    },
    {
      id: 'q2',
      finding_id: 'i1',
      original_path: '/Users/rummager/Downloads/Google Chrome.dmg',
      stored_path: '/Users/rummager/Library/Application Support/Scuttle/Quarantine/q2/Google Chrome.dmg',
      display_name: 'Google Chrome.dmg',
      category: 'installers',
      size: 212 * 1024 ** 2,
      content_hash: 'ab12',
      evidence: [],
      quarantined_unix: now - 13 * DAY,
      expires_unix: now + 1 * DAY,
      status: 'held',
      resolved_unix: null,
    },
  ],
}

export const SPACE: SpaceOverview = {
  volume_total: 994 * 1024 ** 3,
  volume_free: 263 * 1024 ** 3,
  volume_used: 731 * 1024 ** 3,
  // Shaped like a real machine rather than a tidy one: six named areas plus
  // the unaccounted remainder makes seven legend entries, and every category
  // turns up in the tallies. Designing this screen against a shorter fixture
  // is how it came to overflow the window on anyone's actual disk.
  areas: [
    { label: 'App data (Application Support)', bytes: 129 * 1024 ** 3, complete: false },
    { label: 'App data (Caches)', bytes: 82 * 1024 ** 3, complete: false },
    { label: 'Downloads', bytes: 44 * 1024 ** 3, complete: true },
    { label: 'App data (Logs)', bytes: 21 * 1024 ** 3, complete: true },
    { label: 'Movies', bytes: 9 * 1024 ** 3, complete: true },
    { label: 'Documents', bytes: 4 * 1024 ** 3, complete: true },
  ],
  worth_checking: [
    { category: 'heavy_strays', label: 'Heavy strays', bytes: 47 * 1024 ** 3, count: 22 },
    { category: 'installers', label: 'Installers', bytes: 11 * 1024 ** 3, count: 26 },
    { category: 'ghosts', label: 'Ghosts', bytes: 8 * 1024 ** 3, count: 11 },
    { category: 'caches', label: 'Caches', bytes: 6 * 1024 ** 3, count: 7 },
    { category: 'screenshots', label: 'Screenshots', bytes: 2 * 1024 ** 3, count: 412 },
    { category: 'copies', label: 'Copies', bytes: 1024 ** 3 / 2, count: 3 },
  ],
  reclaimable_estimate: 43 * 1024 ** 3,
  has_findings: true,
  summary:
    '731 GB in use.\n\nAround 43 GB may be reclaimable, depending on what you still want.',
}

export const SETTINGS: Settings = {
  scan_roots: [],
  include_developer_debris: false,
  quarantine_retention_days: 14,
  heavy_threshold: 1024 ** 3,
  appearance: 'system',
  reduced_motion: null,
  has_rummaged_before: true,
}

export const CANDIDATES = [...GHOSTS, ...SCREENSHOTS, ...INSTALLERS, ...HEAVY, ...COPIES, ...CACHES, ...ODDMENTS]
