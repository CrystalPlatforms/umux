// parityFixture — the parity lock (#128, phase 5b).
//
// The plan's UI inventory (umux-v1.8.0-plan.md, Appendix — the PO's full-1:1
// decision, 2026-10-27/11-02) becomes DATA here: every UI action the app
// ships maps to its canonical command and gateway op. The suite fails when
// the two halves drift:
//
//   1. every fixture row's op is actually SERVED — layout ops through the
//      real applyGatewayOp switch (unknownOp = failure), live ops through
//      the real handleLiveOp (returns false = failure);
//   2. every catalog op (LAYOUT_OPS, LIVE_OPS) is claimed by a row — an op
//      that exists but no UI action maps to is exactly the drift story 5
//      exists to prevent;
//   3. the UI-only exceptions stay DOCUMENTED — an action that is
//      meaningless without a screen names itself here instead of sneaking
//      into the catalog.
//
// Adding a UI action without a mapping now fails this suite (direction 2:
// the implementer adds the op to a catalog; no row claims it). Adding a row
// whose op is not served fails direction 1. The inventory itself is the
// plan Appendix transcription — a UI change that adds an interaction must
// extend BOTH the app and this table, which is the review artifact.

import { describe, expect, it } from 'vitest'
import { applyGatewayOp, LAYOUT_OPS } from './gatewayOps'
import { handleLiveOp, LIVE_OPS, type LiveOpDeps, type PtyEntry } from './liveOps'
import {
  createWorkspace,
  emptyState,
  type WorkspaceState,
} from './workspaces'
import { defaultSettings } from './settings'

// --- the inventory (plan Appendix, verbatim rows) -----------------------------

type FixtureRow = {
  area:
    | 'Workspaces'
    | 'Groups'
    | 'Tabs'
    | 'Panes'
    | 'Settings'
    | 'Sessions / live I/O'
    | 'System actions'
  action: string
  command: string
  op?: string
  uiOnly?: true
}

const FIXTURE: FixtureRow[] = [
  // --- Workspaces (plan Appendix) ---
  { area: 'Workspaces', action: 'Create ("+" / menu / Ctrl+Shift+N)', command: 'workspaces create', op: 'workspaces.create' },
  { area: 'Workspaces', action: 'Switch/activate (row click, Ctrl+Shift+←/→)', command: 'workspaces open', op: 'workspaces.open' },
  { area: 'Workspaces', action: 'Rename (context menu → inline)', command: 'workspaces rename', op: 'workspaces.rename' },
  { area: 'Workspaces', action: 'Delete (× / menu, confirm dialog)', command: 'workspaces delete', op: 'workspaces.delete' },
  { area: 'Workspaces', action: 'Close, keep definition (batch only today)', command: 'workspaces close', op: 'workspaces.close' },
  { area: 'Workspaces', action: 'Pin/unpin (menu)', command: 'workspaces pin / unpin', op: 'workspaces.pin' },
  { area: 'Workspaces', action: 'Pin/unpin (menu) — the unpin half', command: 'workspaces pin / unpin', op: 'workspaces.unpin' },
  { area: 'Workspaces', action: 'Reorder (DRAG-ONLY)', command: 'workspaces reorder', op: 'workspaces.reorder' },
  { area: 'Workspaces', action: 'Move into existing group (drag/menu picker)', command: 'workspaces move --group', op: 'workspaces.move' },
  { area: 'Workspaces', action: 'Move to NEW group (menu picker)', command: 'workspaces move --new-group', op: 'workspaces.move' },
  { area: 'Workspaces', action: 'Set/clear color (menu swatches, 8 palette)', command: 'workspaces color <c> / clear', op: 'workspaces.color' },
  { area: 'Workspaces', action: 'Batch multi-select (Ctrl+click)', command: 'multi-id args on the verbs', op: 'workspaces.pin' },
  { area: 'Workspaces', action: 'Batch pin/unpin/close/delete', command: 'workspaces pin/unpin/close/delete with multi-id args', op: 'workspaces.close' },
  { area: 'Workspaces', action: 'Inspector: agent chips, folder lines, branch labels, cwd', command: 'workspaces list / tabs branches reads', op: 'tabs.branches' },
  // --- Groups ---
  { area: 'Groups', action: 'Create / rename / delete (+subtree confirm)', command: 'groups create / rename / delete', op: 'groups.create' },
  { area: 'Groups', action: 'Create / rename / delete (+subtree confirm) — the rename half', command: 'groups rename', op: 'groups.rename' },
  { area: 'Groups', action: 'Create / rename / delete (+subtree confirm) — the delete half', command: 'groups delete (--subtree confirms)', op: 'groups.delete' },
  { area: 'Groups', action: 'Unpack (dissolve)', command: 'groups unpack', op: 'groups.unpack' },
  { area: 'Groups', action: 'Pin/unpin', command: 'groups pin / unpin', op: 'groups.pin' },
  { area: 'Groups', action: 'Pin/unpin — the unpin half', command: 'groups pin / unpin', op: 'groups.unpin' },
  { area: 'Groups', action: 'Reorder / nest (DRAG-ONLY)', command: 'groups move', op: 'groups.move' },
  { area: 'Groups', action: 'Collapse/expand', command: 'groups collapse / expand', op: 'groups.collapse' },
  { area: 'Groups', action: 'Collapse/expand — the expand half', command: 'groups collapse / expand', op: 'groups.expand' },
  { area: 'Groups', action: 'Set/clear color', command: 'groups color', op: 'groups.color' },
  // --- Tabs ---
  { area: 'Tabs', action: 'Create ("+" / Ctrl+Shift+T)', command: 'tabs create', op: 'tabs.create' },
  { area: 'Tabs', action: 'Create in chosen shell (chevron picker)', command: 'tabs create --shell <s>', op: 'tabs.create' },
  { area: 'Tabs', action: 'Switch (click; NO keyboard cycling today)', command: 'tabs open', op: 'tabs.open' },
  { area: 'Tabs', action: 'Close (× / menu; busy confirm)', command: 'tabs close (multi-id for bulk)', op: 'tabs.close' },
  { area: 'Tabs', action: 'Rename (double-click / menu)', command: 'tabs rename', op: 'tabs.rename' },
  { area: 'Tabs', action: 'Pin/unpin', command: 'tabs pin / unpin', op: 'tabs.pin' },
  { area: 'Tabs', action: 'Pin/unpin — the unpin half', command: 'tabs pin / unpin', op: 'tabs.unpin' },
  { area: 'Tabs', action: 'Reorder (DRAG-ONLY)', command: 'tabs reorder', op: 'tabs.reorder' },
  { area: 'Tabs', action: 'Move to workspace (menu picker, batch)', command: 'tabs move --workspace <w>', op: 'tabs.move' },
  { area: 'Tabs', action: 'Color', command: 'tabs color', op: 'tabs.color' },
  { area: 'Tabs', action: 'Separate split into tabs (menu)', command: 'tabs separate', op: 'tabs.separate' },
  { area: 'Tabs', action: 'Fullscreen chrome-hide (menu / Esc)', command: 'tabs fullscreen on / off', op: 'tabs.fullscreen' },
  { area: 'Tabs', action: 'Send text to terminal', command: 'tabs send', op: 'tabs.send' },
  { area: 'Tabs', action: 'Ports tooltip (hover → copy/open)', command: 'tabs ports + open port', op: 'tabs.ports' },
  { area: 'Tabs', action: 'Branch labels (auto)', command: 'tabs branches', op: 'tabs.branches' },
  // --- Panes ---
  { area: 'Panes', action: 'Split h/v (Ctrl+Shift+H/E, menu)', command: 'panes split --horizontal/--vertical', op: 'panes.split' },
  { area: 'Panes', action: 'Resize (divider DRAG-ONLY)', command: 'panes resize --ratio <n>', op: 'panes.resize' },
  { area: 'Panes', action: 'Close (× / Ctrl+Shift+W)', command: 'panes close', op: 'panes.close' },
  { area: 'Panes', action: 'Focus (click/typing)', command: 'panes focus', op: 'panes.focus' },
  { area: 'Panes', action: 'Zoom/unzoom (button / Ctrl+Shift+Z)', command: 'panes zoom', op: 'panes.zoom' },
  // --- Settings (app-domain keys) ---
  { area: 'Settings', action: 'notifications / agent status / session restore / ports tooltip / tab branch / tab folders', command: 'settings set <key> <bool> (six keys)', op: 'settings.set' },
  { area: 'Settings', action: 'Default shell (picker + custom dialog)', command: 'settings set default-shell <s> / auto', op: 'settings.set' },
  { area: 'Settings', action: 'Sidebar width (DRAG persists)', command: 'settings set sidebar-width <n>', op: 'settings.set' },
  { area: 'Settings', action: 'Open settings.json (footnote)', command: 'settings open (alias: open settings)', op: 'settings.open' },
  { area: 'Settings', action: 'Daemon-domain switches (Storestation on/off/autostart, Core, Core Session)', command: 'existing storestation/core/core-session commands routed to the serving side', uiOnly: true },
  { area: 'Settings', action: 'Settings get (read the values)', command: 'settings get', op: 'settings.get' },
  // --- Sessions / live I/O ---
  { area: 'Sessions / live I/O', action: 'Live panels inspect', command: 'sessions list (app-served mapping)', op: 'sessions.list' },
  { area: 'Sessions / live I/O', action: 'Attach to panel stream', command: 'sessions subscribe / unsubscribe', op: 'sessions.subscribe' },
  { area: 'Sessions / live I/O', action: 'Attach to panel stream — detach', command: 'sessions unsubscribe', op: 'sessions.unsubscribe' },
  { area: 'Sessions / live I/O', action: "Kill a live panel's shell", command: 'sessions kill', op: 'sessions.kill' },
  { area: 'Sessions / live I/O', action: 'Write to a live panel (daemon sessions surface)', command: 'sessions write', op: 'sessions.write' },
  { area: 'Sessions / live I/O', action: 'Resize a live panel (daemon sessions surface)', command: 'sessions resize', op: 'sessions.resize' },
  { area: 'Sessions / live I/O', action: 'Live panel status (daemon sessions surface)', command: 'sessions status', op: 'sessions.status' },
  { area: 'Sessions / live I/O', action: 'Create a headless session (daemon-only)', command: 'sessions create (daemon spawns; the app answers with guidance)', op: 'sessions.create' },
  // --- System actions (full-1:1, PO 2026-11-02) ---
  { area: 'System actions', action: 'Check for updates / Download & restart', command: 'update check / update install', op: 'update.check' },
  { area: 'System actions', action: 'Check for updates / Download & restart — the install half', command: 'update install', op: 'update.install' },
  { area: 'System actions', action: 'Reset umux (two-click arm)', command: 'umux reset (offline: requires --yes)', op: 'reset.run' },
  { area: 'System actions', action: 'Folder line double-click → file manager', command: 'open folder <target>', op: 'open.folder' },
  { area: 'System actions', action: 'Port click → browser', command: 'open port <tab> <port>', op: 'open.port' },
  { area: 'System actions', action: 'Window minimize/maximize/close (header menu)', command: 'window minimize / toggle-maximize / close', op: 'window.minimize' },
  { area: 'System actions', action: 'Window maximize toggle (header menu)', command: 'window toggle-maximize', op: 'window.toggle-maximize' },
  { area: 'System actions', action: 'Window close (header menu)', command: 'window close', op: 'window.close' },
  { area: 'System actions', action: 'Window minimize (header menu)', command: 'window minimize', op: 'window.minimize' },
  { area: 'System actions', action: 'Notification bell (mute)', command: 'settings set notifications-enabled (same switch)', op: 'settings.set' },
  { area: 'System actions', action: 'cmux import wizard', command: 'offline import cmux stays; live import rides the gateway', uiOnly: true },
  { area: 'System actions', action: 'EmptyState "Create workspace"', command: 'workspaces create', op: 'workspaces.create' },
  { area: 'System actions', action: 'Config-fallback banner dismiss', command: 'transient UI state — not persistable, no command (documented as UI-only per the plan Appendix)', uiOnly: true },
  // --- The plan Appendix's UI-only exceptions (verbatim) ---
  { area: 'System actions', action: 'inline rename inputs', command: 'UI-only exception (plan Appendix)', uiOnly: true },
  { area: 'System actions', action: 'drag ghost / insertion-line visuals', command: 'UI-only exception (plan Appendix)', uiOnly: true },
  { area: 'System actions', action: 'sidebar drag handle itself (width is the persisted counterpart)', command: 'UI-only exception — settings set sidebar-width is the persisted counterpart', uiOnly: true },
  { area: 'System actions', action: 'splash / banner transient states', command: 'UI-only exception (plan Appendix)', uiOnly: true },
]

// --- the harness -------------------------------------------------------------

const state = (): WorkspaceState => createWorkspace(emptyState, 'projekt', () => 'gen-1')

const settings = () => defaultSettings

/// Live-op deps where every side effect records instead of performing.
const NO_WINDOW = () => ({
  minimize: async () => {},
  toggleMaximize: async () => {},
  close: async () => {},
})

function liveDeps(overrides: Partial<LiveOpDeps> = {}): LiveOpDeps {
  // Object.assign keeps the REQUIRED shape: Partial's optional-undefined
  // properties intersect away, so the result stays a full LiveOpDeps.
  return Object.assign(
    {
      invoke: async <T,>(): Promise<T> => undefined as unknown as T,
      state,
      settings,
      liveShell: (_panelId: string | null): PtyEntry | null => null,
      applySettingPatch: () => {},
      relaunch: () => {},
      window: NO_WINDOW,
      openPath: async () => {},
      revealItemInDir: async () => {},
      openUrl: async () => {},
      updaterApi: {
        preflight: async () => 'ok',
        check: async () => null,
        install: async () => {},
        relaunch: () => {},
      },
      setFullscreen: () => {},
    } satisfies LiveOpDeps,
    overrides,
  )
}

/// Is a layout op SERVED by the real dispatch? unknownOp = not served;
/// any other answer (usually badParams on the empty params) = served.
function layoutServed(op: string): boolean {
  const outcome = applyGatewayOp(state(), op, {}, () => 'test-id')
  return !(outcome.ok === false && outcome.error.code === 'unknownOp')
}

/// Is a live op SERVED by the real handler? false = fell through to the
/// layout machine (or nowhere).
function liveServed(op: string): boolean {
  return handleLiveOp(liveDeps(), 1, op, {})
}

// --- the suite ---------------------------------------------------------------

describe('parity fixture (#128 — the plan Appendix inventory)', () => {
  it('maps every inventoried UI action to a command', () => {
    for (const row of FIXTURE) {
      expect(row.command, `the "${row.action}" row needs a command`).toBeTruthy()
    }
  })

  it('every fixture op is SERVED by the real dispatch', () => {
    const layout = new Set(LAYOUT_OPS)
    for (const row of FIXTURE) {
      if (row.uiOnly || row.op == null) continue
      if (layout.has(row.op)) {
        expect(
          layoutServed(row.op),
          `"${row.op}" (the "${row.action}" row) answers unknownOp — the fixture names an op the layout machine does not serve`,
        ).toBe(true)
      } else {
        expect(
          liveServed(row.op),
          `"${row.op}" (the "${row.action}" row) is not handled by the live catalog`,
        ).toBe(true)
      }
    }
  })

  it('every catalog op is claimed by a fixture row', () => {
    const claimed = new Set(FIXTURE.filter((r) => r.op != null).map((r) => r.op as string))
    for (const op of LIVE_OPS) {
      expect(
        claimed.has(op),
        `the live catalog serves "${op}" but no UI action maps to it — add the mapping or drop the op`,
      ).toBe(true)
    }
    for (const op of LAYOUT_OPS) {
      expect(
        claimed.has(op),
        `the layout machine serves "${op}" but no UI action maps to it — add the mapping or drop the op`,
      ).toBe(true)
    }
  })

  it('the UI-only exceptions stay documented as data', () => {
    const uiOnly = FIXTURE.filter((r) => r.uiOnly)
    // The plan Appendix names these four groups verbatim; the fixture keeps
    // them visible so a command never grows for a screen-only state by
    // accident.
    for (const must of ['inline rename inputs', 'drag ghost', 'sidebar drag handle', 'splash / banner transient states']) {
      expect(
        uiOnly.some((r) => r.action.includes(must)),
        `the plan's UI-only exception "${must}" must stay in the fixture`,
      ).toBe(true)
    }
    // Every UI-only row still says WHY (its command field is the reason).
    for (const row of uiOnly) {
      expect(row.command, `the UI-only row "${row.action}" needs its reason`).toBeTruthy()
    }
  })

  it('every fixture op stays within the two real catalogs', () => {
    const known = new Set([...LAYOUT_OPS, ...LIVE_OPS])
    for (const row of FIXTURE) {
      if (row.op == null) continue
      expect(
        known.has(row.op),
        `"${row.op}" (the "${row.action}" row) is in NEITHER catalog — a typo or an op that never shipped`,
      ).toBe(true)
    }
  })
})
