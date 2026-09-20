import type { FailureKind, FindingResult, IssueGroup, MoveReport, NextStep } from '@/lib/types'

/**
 * What to say about a move, in Scuttle's voice.
 *
 * One rule governs every sentence here: it is used only when the report
 * actually says the thing happened. "In use" is said only for failures the
 * operating system reported as a sharing or lock violation; "changed since the
 * scan" only for files that really did change; and nothing ever says space was
 * freed, because moving into the drawer frees none.
 */

export interface Outcome {
  /** One or two sentences: what happened. */
  headline: string
  tone: 'plain' | 'warn'
  /** Findings for which "try the rest again" could do something. */
  retryIds: string[]
  /** Findings that have changed and want to be looked at again. */
  reviewIds: string[]
  /** Whether there is more to see than the headline. */
  hasDetails: boolean
}

const CHANGED: ReadonlySet<FailureKind> = new Set(['stale', 'replaced', 'missing'])

/** How many things failed for each reason, across every finding. */
export function tallyKinds(report: MoveReport): Map<FailureKind, number> {
  const kinds = new Map<FailureKind, number>()
  for (const finding of report.findings) {
    for (const group of finding.issues) {
      kinds.set(group.kind, (kinds.get(group.kind) ?? 0) + group.count)
    }
  }
  return kinds
}

function sum(kinds: Map<FailureKind, number>, wanted: Iterable<FailureKind>): number {
  let total = 0
  for (const kind of wanted) total += kinds.get(kind) ?? 0
  return total
}

function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many
}

function num(n: number): string {
  return n.toLocaleString('en-US')
}

/** "files" when everything that moved was a folder's files, otherwise "items". */
function nounFor(report: MoveReport, n: number): string {
  const movers = report.findings.filter((f) => f.moved > 0)
  const allFiles = movers.length > 0 && movers.every((f) => f.unit === 'files')
  return allFiles ? plural(n, 'file', 'files') : plural(n, 'item', 'items')
}

function leadFor(report: MoveReport): string | null {
  const moved = report.moved_files
  if (moved === 0) return null
  const movers = report.findings.filter((f) => f.moved > 0)
  const only = movers.length === 1 ? movers[0]! : null
  if (only && only.unit === 'item' && moved === 1) {
    return `Moved ${only.display_name} into Drawer.`
  }
  return `Moved ${num(moved)} ${nounFor(report, moved)} into Drawer.`
}

/**
 * The reasons, most useful first. Each is included only if it happened.
 */
function reasonsFor(report: MoveReport): string[] {
  const kinds = tallyKinds(report)
  const reasons: string[] = []
  const movedNone = report.moved_files === 0
  const findings = report.findings

  const inUse = kinds.get('in_use') ?? 0
  if (inUse > 0) reasons.push(`Skipped ${num(inUse)} that ${plural(inUse, 'was', 'were')} in use.`)

  const changed = sum(kinds, CHANGED)
  if (changed > 0) {
    const files = findings.some((f) => f.unit === 'files')
    reasons.push(
      files
        ? 'Some files changed since the scan. Review them again.'
        : plural(changed, 'That changed since the scan. Review it again.', 'Some things changed since the scan. Review them again.'),
    )
  }

  const denied = kinds.get('access_denied') ?? 0
  if (denied > 0) {
    const onlyFolder =
      movedNone && findings.length === 1 && findings[0]!.unit === 'files' && denied === (kinds.get('access_denied') ?? 0)
    reasons.push(
      onlyFolder
        ? "Scuttle couldn't access this folder."
        : `Scuttle couldn't access ${num(denied)}.`,
    )
  }

  if ((kinds.get('no_space') ?? 0) > 0) {
    reasons.push("The drawer's drive is full. Free some space, then try again.")
  }

  const crossDrive = kinds.get('cross_volume_refused') ?? 0
  if (crossDrive > 0) {
    reasons.push(
      `Scuttle left ${num(crossDrive)} where ${plural(crossDrive, 'it is', 'they are')}: moving to another drive would lose details.`,
    )
  }

  const other = sum(kinds, [
    'collision',
    'read_only',
    'path_too_long',
    'unsupported',
    'unsafe',
    'other',
  ])
  if (other > 0) reasons.push(`${num(other)} couldn't be moved.`)

  // Scuttle's own gate, for findings it declined before touching anything.
  const said = new Set<string>()
  for (const finding of findings) {
    const message = finding.refusal?.message
    if (message && !said.has(message)) {
      said.add(message)
      reasons.push(message)
    }
  }
  return reasons
}

export function describeReport(report: MoveReport): Outcome {
  const retryIds = report.findings
    .filter((f) => f.retryable && f.status !== 'moved')
    .map((f) => f.finding_id)
  const reviewIds = report.findings
    .filter((f) => f.needs_refresh || f.issues.some((g) => CHANGED.has(g.kind)))
    .map((f) => f.finding_id)
  const hasDetails =
    report.findings.some((f) => f.issues.length > 0 || f.refusal !== null) ||
    report.notice !== null ||
    report.remaining > 0

  if (report.notice) {
    return { headline: report.notice, tone: 'warn', retryIds, reviewIds, hasDetails: true }
  }

  if (report.outcome === 'cancelled') {
    const moved = report.moved_files
    const left = report.remaining
    const headline =
      moved === 0
        ? 'Stopped before anything moved.'
        : `Stopped. Moved ${num(moved)} ${nounFor(report, moved)} into Drawer; ${num(left)} left where ${plural(left, 'it was', 'they were')}.`
    return { headline, tone: 'plain', retryIds, reviewIds, hasDetails }
  }

  const lead = leadFor(report)
  const reasons = reasonsFor(report)

  if (report.outcome === 'completed' && reasons.length === 0) {
    return {
      headline: lead ?? 'Nothing needed moving.',
      tone: 'plain',
      retryIds,
      reviewIds,
      hasDetails,
    }
  }

  // Two clauses at most; the rest lives in the details.
  const parts = [lead, ...reasons.slice(0, lead ? 1 : 2)].filter((p): p is string => Boolean(p))
  const headline =
    parts.length > 0 ? parts.join(' ') : 'Nothing moved, and Scuttle is not sure why.'
  return { headline, tone: 'warn', retryIds, reviewIds, hasDetails }
}

/** What a person could do about a kind of failure, as a sentence. */
export function adviceFor(step: NextStep): string {
  switch (step) {
    case 'refresh':
      return 'Review these again to get a fresh look.'
    case 'retry':
      return 'These can be tried again.'
    case 'free_space':
      return 'Free some space on the drive that holds the drawer, then try again.'
    case 'close_app':
      return 'Close whatever is using them, then try again.'
    case 'leave_alone':
      return 'Scuttle leaves these alone.'
  }
}

/** One issue, in plain words, for the details view. */
export function issueLine(group: IssueGroup, finding: FindingResult): string {
  const what = finding.unit === 'files' ? plural(group.count, 'file', 'files') : plural(group.count, 'item', 'items')
  return `${num(group.count)} ${what}: ${group.explanation}`
}

/** The code and phase, small, for anyone who needs to look closer. */
export function technicalLine(group: IssueGroup): string {
  const code = group.os_code === null ? 'no OS code' : `OS error ${group.os_code}`
  return `${group.kind.replace(/_/g, ' ')} · while ${group.phase.replace(/_/g, ' ')} · ${code}`
}

/**
 * A block of text that can be copied into a bug report: what happened, where,
 * with which codes — and file *names* only, never paths, never contents.
 */
export function diagnostics(report: MoveReport): string {
  const lines = [
    'Scuttle move report',
    `outcome: ${report.outcome}`,
    `moved: ${report.moved_files} · skipped: ${report.skipped} · failed: ${report.failed} · not attempted: ${report.remaining}`,
  ]
  for (const finding of report.findings) {
    lines.push('', `${finding.display_name} (${finding.status})`)
    lines.push(`  moved ${finding.moved}, skipped ${finding.skipped}, failed ${finding.failed}`)
    if (finding.refusal) lines.push(`  refused: ${finding.refusal.code} — ${finding.refusal.message}`)
    for (const group of finding.issues) {
      lines.push(
        `  ${group.kind} × ${group.count} — phase: ${group.phase}, os code: ${group.os_code ?? 'none'}`,
      )
      if (group.samples.length > 0) lines.push(`    e.g. ${group.samples.join(', ')}`)
    }
  }
  if (report.notice) lines.push('', report.notice)
  return lines.join('\n')
}
