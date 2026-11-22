// transferWizard — the pure layer behind the Desktop↔Terminal import wizard
// (the Terminal counterpart of the cmux wizard's importWizard.ts).
//
// Assumptions encoded here (TDD plan, 2026-11-22):
// - The raw store doc rides serde camelCase, so a source state is exactly
//   { workspaces: Workspace[]; groups: Group[]; order: string[] } — the
//   persisted shapes from workspaces.ts (no runtime-only records).
// - The scope filter is FULL-FIDELITY (PO decision 2026-11-22): split trees
//   survive. Categories only REMOVE:
//     workspaces off  → no rows import; order keeps group ids only;
//     grouping off    → groups dropped, workspaces lose their groupId;
//     directories off → every panel's workingDirectory stripped (sshTarget
//                       untouched — not a cmux-scope concept);
//     tabsLayout off  → each workspace keeps only its FIRST tab (with its
//                       split layout — fidelity rule), stripped of its name;
//                       zero-tab workspaces stay zero-tab.
// - The input is never mutated; the output is a fresh object.
// - NOT tested here: I/O (none — pure), the Rust merge (store_core), the
//   wizard component (ImportWizard.test.tsx).

import { describe, expect, it } from 'vitest'
import { fullImportScope } from './importWizard'
import { splitLeaf, type LayoutNode } from './PaneLayout'
import type { Group, Workspace } from './workspaces'
import { scopeWorkspaceState, type TransferSourceState } from './transferWizard'

const split = splitLeaf({ kind: 'leaf', id: 'leaf-1' }, 'leaf-1', 'horizontal', {
  splitId: 'split-1',
  newLeafId: 'leaf-2',
}) as Extract<LayoutNode, { kind: 'split' }>

const ws = (id: string, over: Partial<Workspace> = {}): Workspace => ({
  id,
  name: `ws-${id}`,
  tabs: [{ id: `${id}-tab-1`, layout: { kind: 'leaf' as const, id: `${id}-leaf-1` } }],
  ...over,
})

const group = (id: string, over: Partial<Group> = {}): Group => ({
  id,
  name: `group-${id}`,
  ...over,
})

const source = (over: Partial<TransferSourceState> = {}): TransferSourceState => ({
  workspaces: [ws('a'), ws('b')],
  groups: [group('g1')],
  order: ['g1', 'a', 'b'],
  ...over,
})

describe('transferWizard: scopeWorkspaceState', () => {
  it('a full scope keeps everything — the doc passes through unharmed', () => {
    const doc = source({
      workspaces: [
        ws('a', {
          groupId: 'g1',
          color: '#2563eb',
          tabs: [
            { id: 't1', layout: split, name: 'main', shell: '/bin/zsh' },
            { id: 't2', layout: { kind: 'leaf', id: 'l2' }, name: 'second' },
          ],
          panels: [
            { id: 'leaf-1', workingDirectory: '/tmp/a', sshTarget: 'host' },
            { id: 'leaf-2', workingDirectory: '/tmp/b' },
          ],
        }),
        ws('b'),
      ],
    })
    expect(scopeWorkspaceState(doc, fullImportScope)).toEqual(doc)
  })

  it('workspaces off drops the rows; the order keeps only the group ids', () => {
    const out = scopeWorkspaceState(source(), { ...fullImportScope, workspaces: false })
    expect(out.workspaces).toEqual([])
    expect(out.groups).toEqual([group('g1')])
    expect(out.order).toEqual(['g1'])
  })

  it('grouping off drops the groups and strips groupId from the rows', () => {
    const doc = source({
      workspaces: [ws('a', { groupId: 'g1' }), ws('b')],
      order: ['a', 'g1', 'b'],
    })
    const out = scopeWorkspaceState(doc, { ...fullImportScope, grouping: false })
    expect(out.groups).toEqual([])
    expect(out.workspaces.map((w) => w.groupId)).toEqual([undefined, undefined])
    expect('groupId' in out.workspaces[0]).toBe(false)
    expect(out.order).toEqual(['a', 'b'])
  })

  it('directories off strips every panel cwd but keeps the splits and ssh targets', () => {
    const doc = source({
      workspaces: [
        ws('a', {
          tabs: [{ id: 't1', layout: split, name: 'main' }],
          panels: [
            { id: 'leaf-1', workingDirectory: '/tmp/a', sshTarget: 'host' },
            { id: 'leaf-2', workingDirectory: '/tmp/b' },
          ],
        }),
      ],
    })
    const out = scopeWorkspaceState(doc, { ...fullImportScope, directories: false })
    expect(out.workspaces[0].panels).toEqual([
      { id: 'leaf-1', sshTarget: 'host' },
      { id: 'leaf-2' },
    ])
    expect(out.workspaces[0].tabs?.[0].layout).toEqual(split)
  })

  it('tabsLayout off keeps only the first tab, unnamed, WITH its split layout', () => {
    const doc = source({
      workspaces: [
        ws('a', {
          tabs: [
            { id: 't1', layout: split, name: 'main', shell: '/bin/zsh' },
            { id: 't2', layout: { kind: 'leaf', id: 'l2' }, name: 'second' },
          ],
        }),
        ws('z', { tabs: [] }),
      ],
    })
    const out = scopeWorkspaceState(doc, { ...fullImportScope, tabsLayout: false })
    expect(out.workspaces[0].tabs).toEqual([{ id: 't1', layout: split, shell: '/bin/zsh' }])
    expect(out.workspaces[1].tabs).toEqual([])
  })

  it('the source doc is never mutated', () => {
    const doc = source({ workspaces: [ws('a', { groupId: 'g1' })] })
    const snapshot = JSON.stringify(doc)
    scopeWorkspaceState(doc, { ...fullImportScope, grouping: false, tabsLayout: false })
    expect(JSON.stringify(doc)).toBe(snapshot)
  })
})
