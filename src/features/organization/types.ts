export type OrganizationKind = 'screenshots' | 'installers'
export type OrganizationGrouping = 'month' | 'together'
export interface Opportunity {
  id: string
  path: string
  root: string
  kind: OrganizationKind
  identity: { size: number }
}
export interface OrganizationInventory { items: Opportunity[]; partial: boolean; skipped_locations: number }
export interface OrganizationDestination { id: string; path: string }
export interface OrganizationPreference { destination: OrganizationDestination; grouping: OrganizationGrouping }
export interface OrganizationFile {
  opportunity: Opportunity
  destination: string
  note: string | null
  cross_volume: boolean
}
export interface OrganizationPlan { id: string; kind: OrganizationKind; preference: OrganizationPreference; items: OrganizationFile[] }
export interface OrganizationRecord {
  file: OrganizationFile
  status: 'pending' | 'moving' | 'moved' | 'failed' | 'cancelled' | 'undoing' | 'undone' | 'attention'
  note: string | null
}
export interface OrganizationBatch {
  id: string
  created_unix: number
  kind: OrganizationKind
  preference: OrganizationPreference
  items: OrganizationRecord[]
  running: boolean
  undoing: boolean
  revision: number
  error: string | null
}
