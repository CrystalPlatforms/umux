// tabPorts — pure helpers behind the hover-pulled listening-ports tooltip
// (v1.0 Phase 15 / #42).
//
// Assumptions encoded by these tests:
//  - formatPorts renders the WHOLE tooltip body: ports joined with the app's
//    " · " separator, ascending already guaranteed by the backend; an EMPTY
//    list must render the explicit "No listening ports" state so a quiet
//    tooltip is never ambiguous.
//  - localPtyIds picks ONE backend handle per panel of the hovered tab,
//    keeping only LOCAL shells (SSH panels are skipped — the local process
//    there is always just the ssh client, same reasoning as #28/#41's cwd
//    and presence queries); panels with no live handle yet contribute
//    nothing but never break the call.
//  - NOT tested here: the WorkspaceShell wiring (hover handler counts its
//    invokes and renders the tooltip in WorkspaceShell.test.tsx), and the
//    per-OS socket enumeration itself (cargo tests in src-tauri).

import { describe, it, expect } from 'vitest'
import { formatPorts, localPtyIds, unionPorts, type PtyInfo } from './tabPorts'

describe('formatPorts', () => {
  // T-F1 (#42 AC2): a tab running dev servers on two ports shows both.
  it('joins several ports with the middle-dot separator', () => {
    expect(formatPorts([8000, 5173])).toBe('8000 · 5173')
  })

  // T-F2 (#42 AC3): the empty answer must be EXPLICIT, never blank space.
  it('shows an explicit no-ports state for an empty list', () => {
    expect(formatPorts([])).toBe('No listening ports')
  })

  it('renders a single port bare', () => {
    expect(formatPorts([3000])).toBe('3000')
  })
})

describe('localPtyIds', () => {
  const ptys = new Map<string, PtyInfo>([
    ['panel-a', { kind: 'local', id: 42 }],
    ['panel-b', { kind: 'ssh', id: 7 }],
  ])

  // T-F3 (#42 assumption — SSH tabs cannot be answered locally).
  it('keeps only local panel handles, in tab order', () => {
    expect(localPtyIds(['panel-a', 'panel-b'], ptys)).toEqual([42])
  })

  // T-F4 (a panel whose shell has not reported open yet, or already died —
  // contributes nothing instead of crashing or inventing a handle).
  it('skips panels without a live handle', () => {
    expect(localPtyIds(['panel-missing', 'panel-b'], ptys)).toEqual([])
    expect(localPtyIds([], ptys)).toEqual([])
  })
})

describe('unionPorts', () => {
  // T-F5 (HITL 2026-08-27 — workspace tooltip): a workspace answer is the
  //   UNION of its tabs' answers, sorted and deduplicated — two tabs that
  //   somehow share a port still show it once.
  it('unions, sorts and deduplicates the tabs answers', () => {
    expect(unionPorts([[8000, 5173], [5173, 3000]])).toEqual([3000, 5173, 8000])
  })

  it('tolerates empty groups and an empty union', () => {
    expect(unionPorts([[8000], []])).toEqual([8000])
    expect(unionPorts([])).toEqual([])
    expect(unionPorts([[], []])).toEqual([])
  })
})
