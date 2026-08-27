// tabPorts — pure helpers behind the hover-pulled listening-ports tooltip
// (v1.0 Phase 15 / #42). Kept free of React and invoke so the display rules
// and the local-handle selection are testable without a running shell; the
// WorkspaceShell wiring that actually calls `tab_ports` on hover lives next
// to the tab row's handlers.

/// A panel's backend handle, mirroring WorkspaceShell's PtyEntry map values.
export interface PtyInfo {
  kind: 'local' | 'ssh'
  id: number
}

/// The tooltip body for one hovered tab: the backend already returns the
/// ports ascending and deduplicated, so this only decides the SEPARATOR and
/// the explicit empty state (#42: a silent tooltip must never be ambiguous).
export function formatPorts(ports: number[]): string {
  if (ports.length === 0) return 'No listening ports'
  return ports.join(' · ')
}

/// Backend PTY handles for one tab's panels: LOCAL shells only (an SSH
/// panel's local process is just the ssh client — it says nothing about
/// remote listeners), in tab order, silently skipping panels whose shell
/// has not reported open or is already gone.
export function localPtyIds(
  panelIds: string[],
  ptys: ReadonlyMap<string, PtyInfo>,
): number[] {
  const out: number[] = []
  for (const panelId of panelIds) {
    const entry = ptys.get(panelId)
    if (entry && entry.kind === 'local') out.push(entry.id)
  }
  return out
}

/// A WORKSPACE tooltip's answer (HITL 2026-08-27): the union of every tab's
/// port list, ascending, each port once — the same canonical form the
/// backend guarantees per tab.
export function unionPorts(groups: number[][]): number[] {
  const out: number[] = []
  for (const ports of groups) out.push(...ports)
  out.sort((a, b) => a - b)
  return out.filter((p, i) => i === 0 || p !== out[i - 1])
}
