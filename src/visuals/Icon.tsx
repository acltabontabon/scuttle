/** Small interface icons; the hand-drawn glyphs still carry Scuttle's personality. */
export function Icon({ name, size = 18 }: { name: 'scan' | 'findings' | 'drawer' | 'space' | 'settings' | 'arrow' | 'shield' | 'search'; size?: number }) {
  const paths = {
    scan: <><circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 4 4M8 10.5h5m-2.5-2.5v5" /></>,
    findings: <><rect x="3" y="3" width="7" height="7" rx="2" /><rect x="14" y="3" width="7" height="7" rx="2" /><rect x="3" y="14" width="7" height="7" rx="2" /><rect x="14" y="14" width="7" height="7" rx="2" /></>,
    drawer: <><path d="m5 4-3 9v7h20v-7l-3-9ZM2 13h6l2 3h4l2-3h6M8 8h8" /></>,
    space: <><circle cx="12" cy="12" r="9" /><path d="M12 3v9h9M12 12l-6 6" /></>,
    settings: <><path d="M4 6h16M4 12h16M4 18h16" /><circle cx="9" cy="6" r="2" fill="var(--paper-raised)" /><circle cx="15" cy="12" r="2" fill="var(--paper-raised)" /><circle cx="9" cy="18" r="2" fill="var(--paper-raised)" /></>,
    arrow: <path d="M4 12h16m-6-6 6 6-6 6" />,
    shield: <><path d="m12 3 8 3v6c0 4-4 7-8 9-4-2-8-5-8-9V6Z" /><path d="m8 12 3 3 5-6" /></>,
    search: <><circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 4 4" /></>,
  }
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[name]}</svg>
}
