// cmuxParity — the TS half of the shared-fixture parity suite (#63).
//
// The Rust importer (store_core/src/cmux_import.rs) is the SECOND
// implementation; per the PO decision of 2026-08-30 neither implementation is
// refactored into the other — parity is ENFORCED here instead. Both sides
// process the SAME fixtures (cmux-config.json + cmux-session.json) against
// the SAME live seed (cmux-parity-live.json) and must reproduce the SAME
// golden (cmux-plan-golden.json): the parse plan verbatim, and the
// collision-resolved preview tree (names, order, groups, suffixes, tab
// counts). `cargo test -p store_core --test cmux_parity` runs the Rust twin —
// drift on either side breaks its own suite before a release ships.
//
// Assumptions encoded:
// - The golden's `plan` is parseCmuxSources' output, JSON-exact (source ids,
//   ` from cmux`-free titles, camelCase fields).
// - The golden's `preview` is buildImportPreviewTree's output over the live
//   seed — runtime-only records are irrelevant to it (applyImportPlan
//   restores them), so the seed carries just the persisted shape plus empty
//   runtime defaults.

import { describe, expect, it } from 'vitest'
import { parseCmuxSources } from './cmuxImport'
import { buildImportPreview, buildImportPreviewTree } from './importWizard'
import type { WorkspaceState } from './workspaces'
import sessionFixture from './fixtures/cmux-session.json'
import configFixture from './fixtures/cmux-config.json'
import liveFixture from './fixtures/cmux-parity-live.json'
import golden from './fixtures/cmux-plan-golden.json'

const plan = parseCmuxSources(
  JSON.stringify(configFixture),
  JSON.stringify(sessionFixture),
)

const live = {
  ...(liveFixture as unknown as Pick<
    WorkspaceState,
    'workspaces' | 'groups' | 'order'
  >),
  activeId: null,
  openIds: [],
  activeTabId: {},
  activePanelId: {},
  zoomedPanelId: {},
} as WorkspaceState

describe('cmux import parity (#63)', () => {
  it('the TypeScript plan matches the committed golden', () => {
    expect(plan).toEqual(golden.plan)
  })

  it('the TypeScript import result matches the committed golden tree', () => {
    const preview = buildImportPreview(live, plan)
    expect(buildImportPreviewTree(preview)).toEqual(golden.preview)
  })
})
