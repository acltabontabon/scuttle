import type { OrganizationBatch, OrganizationInventory, OrganizationKind, OrganizationGrouping, OrganizationPreference, OrganizationPlan } from '@/features/organization/types'
import { api } from '@/lib/ipc'

/** Browser-only fixtures for settings that normally load from the native app. */
export function installPreviewApi() {
  const originals = { ...api }
  const inventory: OrganizationInventory = { partial: false, skipped_locations: 0, items: [
    { id: 'organize-shot-1', path: '/Users/demo/Desktop/Screenshot 2026-10-08.png', root: '/Users/demo/Desktop', kind: 'screenshots', identity: { size: 238400 } },
    { id: 'organize-shot-2', path: '/Users/demo/Documents/Screenshot 2026-10-09.png', root: '/Users/demo/Documents', kind: 'screenshots', identity: { size: 428900 } },
    { id: 'organize-setup', path: '/Users/demo/Downloads/FigmaSetup.dmg', root: '/Users/demo/Downloads', kind: 'installers', identity: { size: 82400000 } },
  ] }
  const plans = new Map<string, OrganizationPlan>()
  const batches: OrganizationBatch[] = []
  const preferences = new Map<OrganizationKind, OrganizationPreference>()
  const destination = (kind: OrganizationKind) => ({ id: kind, path: kind === 'screenshots' ? '/Users/demo/Pictures/Screenshots' : '/Users/demo/Downloads/Installers' })
  const preview = 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100"><rect width="160" height="100" fill="#f4efe6"/><rect x="10" y="10" width="140" height="80" rx="5" fill="#fff"/><rect x="10" y="10" width="140" height="13" rx="5" fill="#a6b6a0"/><path d="M22 37h65m-65 10h110m-110 10h92m-92 10h105m-105 10h54" stroke="#cbc4b9" stroke-width="4"/></svg>')
  Object.assign(api, {
    organizationInventory: async () => ({ ...inventory, items: inventory.items.filter((o) => !batches.some((b) => b.items.some((r) => r.file.opportunity.id === o.id && r.status === 'moved'))) }),
    organizationPreference: async (kind: OrganizationKind) => preferences.get(kind) ?? { destination: destination(kind), grouping: kind === 'screenshots' ? 'month' : 'together' },
    chooseOrganizationDestination: async () => ({ id: 'custom', path: '/Users/demo/Pictures/Collected' }),
    organizationThumbnail: async () => preview,
    planOrganization: async (kind: OrganizationKind, ids: string[], destinationId: string, grouping: OrganizationGrouping) => {
      const folder = destinationId === 'custom' ? { id: 'custom', path: '/Users/demo/Pictures/Collected' } : destination(kind)
      const plan: OrganizationPlan = { id: `preview-${plans.size}`, kind, preference: { destination: folder, grouping }, items: inventory.items.filter((o) => ids.includes(o.id)).map((o) => ({ opportunity: o, destination: `${folder.path}/${grouping === 'month' ? '2026-10/' : ''}${o.path.split('/').pop()}`, note: null, cross_volume: false })) }
      plans.set(plan.id, plan); return plan
    },
    startOrganization: async (id: string) => {
      const plan = plans.get(id)!
      const batch: OrganizationBatch = { id: `batch-${batches.length}`, created_unix: Date.now() / 1000, kind: plan.kind, preference: plan.preference, items: plan.items.map((file) => ({ file, status: 'moved', note: null })), running: false, undoing: false, revision: 1, error: null }
      batches.unshift(batch); preferences.set(plan.kind, plan.preference); return batch
    },
    undoOrganization: async (id: string) => {
      const batch = batches.find((b) => b.id === id)!
      batch.items = batch.items.map((r) => ({ ...r, status: 'undone' })); batch.undoing = true; batch.revision += 1
      return { ...batch }
    },
    organizationStatus: async (id: string) => batches.find((b) => b.id === id),
    organizationHistory: async (offset = 0) => batches.slice(offset, offset + 20),
    cancelOrganization: async () => {},
    revealOrganization: async () => {},

    suggestedRoots: async () => [
      { label: 'Downloads', path: '/Users/demo/Downloads' },
      { label: 'Desktop', path: '/Users/demo/Desktop' },
      { label: 'Application support', path: '/Users/demo/Library/Application Support' },
      { label: 'Caches', path: '/Users/demo/Library/Caches' },
    ],
    developerRoots: async () => ['/Users/demo/Projects'],
    ignored: async () => [],
    about: async () => ({ version: 'preview · sample data', platform: 'Browser preview', quarantine_root: '/Users/demo/Scuttle/Drawer' }),
  })
  return () => Object.assign(api, originals)
}
