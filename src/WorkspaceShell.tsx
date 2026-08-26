// WorkspaceShell — the workspace switcher UI (Phase 6 / #7).
//
//   - On mount: load workspace definitions from the Rust WorkspaceStore via
//     `load_workspaces`; seed state (activeId = first workspace, or null).
//   - Create: the header "+" reveals a name input; committing creates a
//     workspace, makes it active, and persists via `save_workspaces`.
//   - Rename: the pencil icon per row reveals an inline edit; persists on commit.
//   - Switch: clicking a workspace sets it active. Every workspace's panel
//     stays mounted (hidden when inactive) so each keeps its own shell state.
//   - Context menu (right-click): "New workspace" everywhere; the header menu
//     additionally exposes window Minimize / Maximize / Close (Tauri window API).
//
// UI glue verified by Adam on Ubuntu/Wayland; the testable core lives in
// ./workspaces (pure state) and the Rust WorkspaceStore (persistence).

import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { TerminalSurface } from './TerminalSurface'
import { EmptyState } from './EmptyState'
import {
  emptyState,
  createWorkspace,
  renameWorkspace,
  openWorkspace,
  closeWorkspace,
  deleteWorkspace,
  moveWorkspace,
  setWorkspacePinned,
  addTab,
  closeTab,
  switchTab,
  renameTab,
  setTabPinned,
  activeTabOf,
  splitPanel,
  resizePanel,
  closePanel,
  focusPanel,
  activePanelOf,
  bootState,
  panelIdsOf,
  upsertPanelCwd,
  type Panel,
  type Workspace,
  type WorkspaceState,
} from './workspaces'
import { boxes, leafIds, type LayoutNode, type Orientation, type Container } from './PaneLayout'
import { matchShortcut } from './shortcuts'
import { NotificationMuteButton } from './NotificationMuteButton'
import { AgentStatusIndicator } from './AgentStatusIndicator'
import { AgentStatusMachine, type AgentStatus } from './agentStatus'
import { isAiCliProcess } from './aiCli'
import { SettingsDialog } from './SettingsDialog'
import { CloseConfirmDialog } from './CloseConfirmDialog'
import { coerceSettings, defaultSettings, type Settings } from './settings'

// --- Icons (inline SVG, no extra dependency) ---------------------------------

type IconProps = { className?: string }

function PlusIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 5v14M5 12h14" />
    </svg>
  )
}

function PencilIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </svg>
  )
}

function MinimizeIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
      <path d="M5 12h14" />
    </svg>
  )
}

function MaximizeIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="5" y="5" width="14" height="14" rx="2" />
    </svg>
  )
}

function CloseIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
      <path d="M6 6l12 12M18 6L6 18" />
    </svg>
  )
}

function SidebarCollapseIcon({ className }: IconProps) {
  // Panel with a left rail + a left-pointing chevron (collapse to the left).
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M9 3v18" />
      <path d="m15 9-3 3 3 3" />
    </svg>
  )
}

function SidebarExpandIcon({ className }: IconProps) {
  // Panel with a left rail + a right-pointing chevron (expand from the left).
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M9 3v18" />
      <path d="m13 9 3 3-3 3" />
    </svg>
  )
}


function PinIcon({ className }: IconProps) {
  // Pushpin — marks a pinned workspace (#37).
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 17v5" />
      <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1z" />
    </svg>
  )
}

function SplitHorizontalIcon({ className }: IconProps) {
  // A panel split into left | right by a vertical divider.
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M12 3v18" />
    </svg>
  )
}

function SplitVerticalIcon({ className }: IconProps) {
  // A panel split into top / bottom by a horizontal divider.
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M3 12h18" />
    </svg>
  )
}

function SettingsIcon({ className }: IconProps) {
  // Gear — opens the Settings screen (v0.2 Phase 3 / #27).
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.01a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.01a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
    </svg>
  )
}

// --- Context menu state ------------------------------------------------------

type MenuState = {
  x: number
  y: number
  header: boolean
  workspaceId?: string
  // Set when the menu was opened from a TERMINAL TAB (#37 follow-up): the
  // tab-scoped actions (Rename tab / Pin tab / Close tab) show instead of
  // the workspace-list actions.
  tabId?: string
} | null

/// A mouse press that should open the context menu: the right button, or
/// Ctrl+click (the macOS right-click). macOS trackpads additionally deliver a
/// two-finger click as a right mousedown WITHOUT a DOM `contextmenu` event,
/// so the menu must open here too, not only on `onContextMenu`.
function isMenuPress(e: React.MouseEvent): boolean {
  return e.button === 2 || (e.ctrlKey && e.button === 0)
}

/// Human-readable label for a tab (confirmations, menu): the workspace's
/// name plus the tab's own name, falling back to its positional number.
function tabMenuLabel(ws: Workspace | undefined, tabId: string, index: number): string {
  const tab = ws?.tabs?.find((t) => t.id === tabId)
  return `${ws?.name ?? 'workspace'} · ${tab?.name ?? `Tab ${index + 1}`}`
}

// PanelSurfaces — renders a workspace's split tree as a FLAT list of
// absolutely-positioned terminal panes plus divider bars (v0.2 Phase 1 / #25).
// Every pane is keyed by its stable leaf id inside this one stable parent, so
// tree-shape changes (split, close, resize) only reposition surfaces — a
// terminal is never remounted, and its shell/content survives. This is the
// fix for close-remounts: the earlier recursive flex tree re-created DOM
// nodes whenever a split collapsed, killing every shell below it.
//
// Pane/divider geometry comes from PaneLayout.boxes (pure), measured from the
// live container via ResizeObserver. Divider drags report the split node's id
// plus the node's own extent, so the minimum-size clamp in setRatio is correct
// at any nesting depth.
const DIVIDER_PX = 6

type PanelSurfacesProps = {
  workspaceId: string
  workspaceName: string
  layout: LayoutNode
  activePanelId: string | null
  // Identity of the tree's first leaf — per-leaf panel config (v0.2 Phase 5 /
  // #29) with a legacy fallback: a leaf with its own entry uses it; the first
  // leaf additionally falls back to the config's first entry (the pre-v0.2
  // shape stored one entry for the workspace's only panel).
  firstLeafId: string
  panels: Panel[] | undefined
  onResize: (splitId: string, ratio: number, container: Container) => void
  onResizeEnd: () => void
  onClose: (panelId: string) => void
  onFocusPanel: (panelId: string) => void
  // Per-panel status signals (v0.2 Phase 2 / #26): the shell owns one status
  // machine per panel; surfaces report activity/completion. Rendering lives on
  // the workspace row in the sidebar (Adam's call), not in the panel chrome.
  onPanelActivity: (panelId: string, bytes: number) => void
  onPanelCompletion: (panelId: string) => void
  onPanelViewportResize: (panelId: string) => void
  onPanelUserInput: (panelId: string, submitted: boolean) => void
  // v0.2 Phase 4 / #28 + Phase 5 / #29: a surface reports the backend handle
  // it was assigned (and whether it is a remote/SSH one), so the shell can
  // ask "is a live process running in this panel?" before a close and read
  // the shell process's cwd for the session snapshot.
  onPanelOpened: (panelId: string, ptyId: number, remote: boolean) => void
  // Per-panel statuses (HITL round 3: 3 splits = 3 chips). The workspace row
  // keeps its aggregate; each pane additionally shows its own dot + label.
  statuses: Record<string, AgentStatus>
  // Settings gate (v0.2 Phase 3 / #27): when the agent-status toggle is off,
  // panes render no indicator at all — the machines keep running so flipping
  // the toggle back on is instant and lossless.
  statusEnabled: boolean
}

function PanelSurfaces({ workspaceId, workspaceName, layout, activePanelId, firstLeafId, panels, onResize, onResizeEnd, onClose, onFocusPanel, onPanelActivity, onPanelCompletion, onPanelViewportResize, onPanelUserInput, onPanelOpened, statuses, statusEnabled }: PanelSurfacesProps) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })

  // Track the container's box so pane geometry follows window resizes.
  useEffect(() => {
    const el = containerRef.current
    if (el == null || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(() => {
      const r = el.getBoundingClientRect()
      setSize({ width: r.width, height: r.height })
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const { panes, dividers } = boxes(layout, size, DIVIDER_PX)

  /// A pane's stored config (cwd / SSH target), by leaf id with the
  /// first-leaf legacy fallback described on the `panels` prop. `?? []` keeps
  /// a hand-edited config (no panels array) from crashing the render.
  const metaFor = (paneId: string): Panel | undefined => {
    const list = panels ?? []
    const own = list.find((m) => m.id === paneId)
    if (own != null) return own
    return paneId === firstLeafId ? list[0] : undefined
  }

  // Drag a divider: the pointer's position along the split's axis becomes the
  // ratio over the DIVIDABLE length (axis minus this divider's own gap), so
  // the bar lands under the cursor. Capture/release on window so the drag
  // continues beyond the thin hit area.
  const startDrag = (d: (typeof dividers)[number]) => (e: React.PointerEvent) => {
    e.preventDefault()
    const horizontal = d.orientation === 'horizontal'
    const rect = d.splitRect

    const onMove = (ev: PointerEvent) => {
      const avail = horizontal ? rect.width - DIVIDER_PX : rect.height - DIVIDER_PX
      if (avail <= 0) return
      const pos = horizontal ? ev.clientX - rect.x : ev.clientY - rect.y
      onResize(
        d.id,
        pos / avail,
        // The clamp axis is the space the two panes actually share.
        horizontal
          ? { width: avail, height: rect.height }
          : { width: rect.width, height: avail },
      )
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      onResizeEnd()
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  return (
    <div
      ref={containerRef}
      className="panel-surfaces"
      data-testid={`panel-surfaces-${workspaceId}`}
    >
      {panes.map((p) => {
        // Stable, human-readable panel tag for notifications and the close
        // button — position names (left/right) stop working with N panels.
        const short = p.id.slice(0, 4)
        const meta = metaFor(p.id)
        return (
          <div
            key={p.id}
            className={`surface ${activePanelId === p.id ? 'is-active' : ''}`}
            data-panel-id={p.id}
            style={{ left: p.rect.x, top: p.rect.y, width: p.rect.width, height: p.rect.height }}
            onClick={() => onFocusPanel(p.id)}
          >
            <TerminalSurface
              label={`${workspaceName} · ${short}`}
              sshTarget={meta?.sshTarget}
              cwd={meta?.workingDirectory}
              onActivity={(bytes) => onPanelActivity(p.id, bytes)}
              onCompletion={() => onPanelCompletion(p.id)}
              onViewportResize={() => onPanelViewportResize(p.id)}
              onUserInput={(submitted) => onPanelUserInput(p.id, submitted)}
              onOpened={(ptyId) => onPanelOpened(p.id, ptyId, meta?.sshTarget !== undefined)}
            />
            {statusEnabled && (
              <AgentStatusIndicator status={statuses[p.id] ?? 'idle'} />
            )}
            <PanelCloseButton onClose={() => onClose(p.id)} short={short} />
          </div>
        )
      })}
      {dividers.map((d) => (
        <div
          key={d.id}
          className={`divider divider-${d.orientation}`}
          style={{ left: d.rect.x, top: d.rect.y, width: d.rect.width, height: d.rect.height }}
          role="separator"
          aria-label="Resize panels"
          onPointerDown={startDrag(d)}
        />
      ))}
    </div>
  )
}

type PanelCloseButtonProps = { onClose: () => void; short: string }

function PanelCloseButton({ onClose, short }: PanelCloseButtonProps) {
  return (
    <button
      className="panel-close"
      aria-label={`Close panel ${short}`}
      title="Close panel"
      onClick={(e) => {
        e.stopPropagation()
        onClose()
      }}
    >
      <CloseIcon />
    </button>
  )
}

export function WorkspaceShell() {
  const [state, setState] = useState<WorkspaceState>(emptyState)
  const [creating, setCreating] = useState(false)
  const [draftName, setDraftName] = useState('')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editName, setEditName] = useState('')
  // Tab rename (#37 follow-up): which TAB is being renamed inline (double
  // click or the tab menu) and the draft name.
  const [editingTab, setEditingTab] = useState<{ wsId: string; tabId: string } | null>(null)
  const [editTabName, setEditTabName] = useState('')
  const [menu, setMenu] = useState<MenuState>(null)
  // When the current menu was opened (ms epoch). The same gesture can emit a
  // click/contextmenu right AFTER the mousedown that opened the menu (Linux/
  // Windows right-click, macOS Ctrl+click) — those must not close it again.
  const menuOpenedAtRef = useRef(0)
  const [collapsed, setCollapsed] = useState(false)
  const [draggedId, setDraggedId] = useState<string | null>(null)
  // Feature toggles (v0.2 Phase 3 / #27). Loaded from the Rust SettingsStore
  // on mount; every change is persisted immediately and takes effect live.
  // `muted` is derived from the notifications toggle — the bell button and
  // the Settings switch are one source of truth (supersedes v0.1's
  // session-only mute).
  const [settings, setSettings] = useState<Settings>(defaultSettings)
  const [settingsOpen, setSettingsOpen] = useState(false)
  // Latest settings for event-time readers (the window-close and interval
  // effects hold first-render closures; they must read current values).
  const settingsRef = useRef(settings)
  settingsRef.current = settings
  // Config fallback warning (Phase 18 / #19, AC3). Set when the backend emits
  // `config_fallback` (corrupt/unreadable config -> defaults), so the downgrade
  // is surfaced instead of silent. Dismissible; cleared on dismiss.
  const [fallbackMessage, setFallbackMessage] = useState<string | null>(null)

  // Boot (#27 + #29): workspaces and settings load TOGETHER, and panels mount
  // only after both resolve — the session-restore toggle decides whether a
  // restored panel receives its saved cwd at PTY-open time (the surface
  // consumes cwd once, at mount), so it must be known before the first mount.
  useEffect(() => {
    void Promise.all([
      invoke<{ workspaces: Workspace[] }>('load_workspaces').catch(() => ({
        workspaces: [] as Workspace[],
      })),
      invoke<Settings>('load_settings').catch(() => defaultSettings),
    ]).then(([data, rawSettings]) => {
      setSettings(coerceSettings(rawSettings))
      // bootState keeps each workspace's persisted layout tree (v0.2 / #25 —
      // the split layout round-trips through restart) and seeds a fresh single
      // leaf for pre-v0.2 configs that have none. Every defined workspace
      // starts open with its first panel focused.
      setState(bootState(data.workspaces ?? []))
    })
  }, [])

  /// Apply a settings patch (v0.2 Phase 3 / #27): update local state, persist
  /// to settings.json, and — when the notifications toggle moved — flip the
  /// backend runtime flag every panel's notification thread reads. The change
  /// is live everywhere with no restart.
  const applySettings = (patch: Partial<Settings>) => {
    const next = { ...settings, ...patch }
    setSettings(next)
    invoke('save_settings', { settings: next }).catch((e) =>
      console.error('save_settings failed:', e),
    )
    if (patch.notificationsEnabled !== undefined) {
      void invoke('set_notifications_muted', {
        muted: !patch.notificationsEnabled,
      })
    }
  }

  const muted = !settings.notificationsEnabled

  // Surface a config fallback as a visible warning (AC3). The backend emits
  // `config_fallback` from `load_workspaces` only when the config was corrupt
  // or unreadable; a normal first run (missing file) stays silent.
  useEffect(() => {
    const unlistenP = listen<{ message: string }>('config_fallback', (event) => {
      setFallbackMessage(event.payload.message)
    })
    return () => {
      void unlistenP.then((fn) => fn())
    }
  }, [])

  // Persist the session BEFORE the window goes away (v0.2 Phase 5 / #29):
  // intercept the close request, await a cwd snapshot + save, then destroy
  // the window (destroy bypasses this listener, so there is no loop). The
  // whole detour is one invoke round-trip — imperceptible on quit. All state
  // access goes through refs, so the first render's closure stays valid.
  useEffect(() => {
    let handled = false
    const unlistenP = getCurrentWindow().onCloseRequested(async (event) => {
      if (handled) return
      handled = true
      event.preventDefault()
      try {
        await snapshotAndPersist()
      } finally {
        void getCurrentWindow().destroy()
      }
    })
    return () => {
      void unlistenP.then((fn) => fn())
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Periodic cwd snapshot (#29 HITL follow-up): quitting through paths that
  // never fire onCloseRequested — Ctrl+C on `tauri dev`, Cmd+Q's app-level
  // termination, a crash, a kill — must not lose the session either. One
  // quiet tick every 20s bounds the loss to the last few seconds of `cd`s;
  // snapshotAndPersist skips the disk write entirely when nothing moved, so
  // the steady-state cost is just the (cheap) cwd reads.
  useEffect(() => {
    const timer = window.setInterval(() => {
      void snapshotAndPersist()
    }, 20_000)
    return () => window.clearInterval(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // The bell button and the Settings notifications toggle are the same switch
  // (one persisted source of truth) — flipping either routes through
  // applySettings so settings.json and the backend flag never disagree.
  const toggleMute = () => {
    applySettings({ notificationsEnabled: muted ? true : false })
  }

  // Close the context menu on any click outside it — unless it just opened
  // (see menuOpenedAtRef): the opening gesture's own trailing click/contextmenu
  // events would otherwise close the menu in the same instant.
  useEffect(() => {
    if (menu == null) return
    const close = () => {
      if (Date.now() - menuOpenedAtRef.current < 300) return
      setMenu(null)
    }
    window.addEventListener('click', close)
    window.addEventListener('contextmenu', close)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('contextmenu', close)
    }
  }, [menu])

  const persist = (next: WorkspaceState) => {
    setState(next)
    // A rejected save must at least reach the console — a silent one once hid
    // an argument-name mismatch for an entire release (nothing persisted).
    invoke('save_workspaces', { workspaces: next.workspaces }).catch((e) =>
      console.error('save_workspaces failed:', e),
    )
  }

  // --- Session snapshot (v0.2 Phase 5 / #29) ---------------------------------
  //
  // Reads every live local shell's cwd from the backend (one invoke) and
  // merges it into the persisted panels[] entries, then saves. Called for
  // freshness after layout changes (split / closes) and — critically — on
  // window close, where awaiting it guarantees the snapshot is on disk
  // before the process exits. Remote panels are not queried: the remote cwd
  // is not visible locally (OSC-only policy), so their entry keeps the last
  // known value. A failed read leaves the stored cwd untouched (fallback to
  // v0.1 behavior, never a crash).
  const snapshotAndPersist = async () => {
    // Session-restore toggle off = do not snapshot (#27 HITL follow-up): no
    // cwd reads, no writes — shells just start in the default directory.
    if (!settingsRef.current.sessionRestoreEnabled) return
    const st = stateRef.current
    const queries: Array<{ panelId: string; ptyId: number }> = []
    for (const ws of st.workspaces) {
      if (!st.openIds.includes(ws.id)) continue
      for (const pid of panelIdsOf(st, ws.id)) {
        const entry = ptyIdsRef.current.get(pid)
        if (entry != null && entry.kind === 'local') {
          queries.push({ panelId: pid, ptyId: entry.id })
        }
      }
    }
    let cwds: Array<[string, string]> = []
    if (queries.length > 0) {
      try {
        const answers = await invoke<Array<{ panelId: string; cwd: string | null }>>(
          'panel_cwds',
          { panels: queries },
        )
        cwds = answers
          .filter((a) => a.cwd != null)
          .map((a) => [a.panelId, a.cwd as string] as [string, string])
      } catch (e) {
        console.error('panel_cwds failed:', e)
      }
    }
    // Nothing learned (no live local panels, or every read failed): touching
    // state here would only re-write what the triggering persist already
    // saved — and a synchronous re-render from a pre-persist snapshot would
    // actively revert the layout change. Leave state and file alone.
    if (cwds.length === 0) return
    // Race-proof render: the upserts go in as a FUNCTIONAL update, so React
    // applies them onto whatever state is current when it processes them. A
    // whole-state setState from a base read mid-invoke could revert a layout
    // change that landed in the meantime (same class of bug as the focus
    // yank fixed above); the keyed upserts are safe to replay on any state.
    setState((prev) => {
      let next = prev
      for (const [pid, cwd] of cwds) next = upsertPanelCwd(next, pid, cwd)
      return next
    })
    // The file gets the same keyed upserts applied to the freshest committed
    // state. Any later persist already includes them (they are in state by
    // then), so last-write-wins between concurrent saves never loses the
    // snapshot. Awaited so the window-close path quits only after the write.
    // When every upsert no-ops (nothing moved — the common case on the
    // periodic tick), merged IS the base: skip the write entirely.
    let merged = stateRef.current
    for (const [pid, cwd] of cwds) merged = upsertPanelCwd(merged, pid, cwd)
    if (merged === stateRef.current) return
    await invoke('save_workspaces', { workspaces: merged.workspaces }).catch((e) => {
      console.error('save_workspaces failed:', e)
      return undefined
    })
  }

  // --- Keyboard shortcuts (Phase 11 / #12, story 33) -----------------------
  //
  // A window-level keydown listener in the CAPTURE phase so umux sees the key
  // before xterm's hidden textarea does — otherwise the shell would swallow
  // our combos. matchShortcut returns null for anything that isn't one of our
  // Ctrl+Shift combos (and while editing text), so plain shell shortcuts
  // (Ctrl+C/D/Z/...) fall through untouched (AC3). The effect re-binds on
  // every state change so the dispatchers always see fresh state.
  const defaultWorkspaceName = (s: WorkspaceState): string => {
    const taken = new Set(s.workspaces.map((w) => w.name))
    let n = s.workspaces.length + 1
    let name = `Workspace ${n}`
    while (taken.has(name)) name = `Workspace ${++n}`
    return name
  }

  const cycleWorkspace = (dir: 1 | -1): WorkspaceState => {
    const ids = state.workspaces.map((w) => w.id)
    if (ids.length < 2) return state
    const idx = state.activeId != null ? ids.indexOf(state.activeId) : -1
    const nextIdx = (idx + dir + ids.length) % ids.length
    return openWorkspace(state, ids[nextIdx])
  }

  const dispatchShortcut = (cmd: ReturnType<typeof matchShortcut>) => {
    if (cmd == null) return
    const activeId = state.activeId
    switch (cmd) {
      case 'open-settings':
        // Cmd+, (#39 follow-up): the macOS preferences convention.
        setSettingsOpen(true)
        break
      case 'new-workspace':
        persist(createWorkspace(state, defaultWorkspaceName(state)))
        break
      case 'new-tab':
        // Ctrl+Shift+T (#37 rework): a fresh terminal tab in the active
        // workspace — the browser instinct.
        if (activeId != null) persist(addTab(stateRef.current, activeId))
        break
      case 'next-workspace':
        setState(cycleWorkspace(1))
        break
      case 'prev-workspace':
        setState(cycleWorkspace(-1))
        break
      case 'split-horizontal':
        if (activeId != null) persist(splitPanel(state, activeId, 'horizontal'))
        break
      case 'split-vertical':
        if (activeId != null) persist(splitPanel(state, activeId, 'vertical'))
        break
      case 'close-panel': {
        if (activeId == null) break
        const panel = activePanelOf(state, activeId)
        // v0.2 Phase 4 / #28: the keyboard path asks the same live-process
        // question the X button does — one rule for every close path.
        if (panel != null) requestClosePanel(activeId, panel)
        break
      }
    }
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const cmd = matchShortcut({
        key: e.key,
        ctrlKey: e.ctrlKey,
        altKey: e.altKey,
        shiftKey: e.shiftKey,
        metaKey: e.metaKey,
        activeTag: document.activeElement?.tagName ?? null,
      })
      if (cmd == null) return
      e.preventDefault()
      dispatchShortcut(cmd)
    }
    // Capture phase: intercept before xterm's textarea handles the key.
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state])

  const startCreate = () => {
    setCreating(true)
    setEditingId(null)
  }

  const handleCreate = () => {
    if (draftName.trim() === '') return
    persist(createWorkspace(state, draftName.trim()))
    setDraftName('')
    setCreating(false)
  }

  /// Cancel workspace creation (#37): hide the name form and drop the draft.
  /// The visible × button and the Escape key share this one path.
  const cancelCreate = () => {
    setCreating(false)
    setDraftName('')
  }

  const startRename = (ws: Workspace) => {
    setEditingId(ws.id)
    setEditName(ws.name)
  }

  const commitRename = (id: string) => {
    const name = editName.trim()
    setEditingId(null)
    if (name !== '') persist(renameWorkspace(state, id, name))
  }

  // --- Tab rename + pin (#37 follow-up) ------------------------------------
  // The tab-level twins of the workspace actions: one inline-edit path
  // (double-click or the tab menu), definitions persist on commit, and the
  // pin groups tabs at the top of their bar (setTabPinned keeps the
  // invariant).
  const startTabRename = (wsId: string, tabId: string, current?: string) => {
    // Belt and braces: when the caller does not pass the current name, look
    // it up — the input must always open showing the tab's existing name
    // (HITL: "when I click rename, the input should hold the current name").
    const fallback = stateRef.current.workspaces
      .find((w) => w.id === wsId)
      ?.tabs?.find((t) => t.id === tabId)?.name
    setEditingTab({ wsId, tabId })
    setEditTabName(current ?? fallback ?? '')
  }

  const cancelTabRename = () => {
    setEditingTab(null)
    setEditTabName('')
  }

  const commitTabRename = (wsId: string, tabId: string) => {
    const name = editTabName
    setEditingTab(null)
    setEditTabName('')
    // An empty commit is a CANCEL, not "unname": a nameless tab would fall
    // back to a POSITIONAL number, which reshuffles on pin/reorder (the
    // very bug class this UI just escaped).
    if (name.trim() === '') return
    persist(renameTab(stateRef.current, wsId, tabId, name))
  }

  const renameTabFromMenu = () => {
    const { workspaceId, tabId } = menu ?? {}
    setMenu(null)
    if (workspaceId == null || tabId == null) return
    const tab = stateRef.current.workspaces
      .find((w) => w.id === workspaceId)
      ?.tabs?.find((t) => t.id === tabId)
    if (tab != null) startTabRename(workspaceId, tabId, tab.name)
  }

  const togglePinTabFromMenu = () => {
    const { workspaceId, tabId } = menu ?? {}
    setMenu(null)
    if (workspaceId == null || tabId == null) return
    const tab = stateRef.current.workspaces
      .find((w) => w.id === workspaceId)
      ?.tabs?.find((t) => t.id === tabId)
    persist(setTabPinned(stateRef.current, workspaceId, tabId, tab?.pinned !== true))
  }

  const closeTabFromMenu = () => {
    const { workspaceId, tabId } = menu ?? {}
    setMenu(null)
    if (workspaceId == null || tabId == null) return
    const ws = stateRef.current.workspaces.find((w) => w.id === workspaceId)
    const index = ws?.tabs?.findIndex((t) => t.id === tabId) ?? -1
    const label = tabMenuLabel(ws, tabId, index)
    requestCloseTab(workspaceId, tabId, label)
  }

  const openMenu = (
    e: React.MouseEvent,
    header: boolean,
    workspaceId?: string,
    tabId?: string,
  ) => {
    e.preventDefault()
    e.stopPropagation()
    menuOpenedAtRef.current = Date.now()
    setMenu({ x: e.clientX, y: e.clientY, header, workspaceId, tabId })
  }

  const deleteFromMenu = () => {
    const id = menu?.workspaceId
    setMenu(null)
    if (id == null) return
    // Ask through the shared modal (window.confirm is a no-op returning
    // false inside WKWebView — see the pendingDelete block above).
    const ws = stateRef.current.workspaces.find((w) => w.id === id)
    setPendingDelete({ id, name: ws?.name ?? 'workspace' })
  }

  const splitFromMenu = (orientation: Orientation) => {
    const id = menu?.workspaceId
    setMenu(null)
    if (id == null) return
    splitWorkspace(id, orientation)
  }

  /// Pin/unpin from the context menu (#37): flips the workspace's pinned flag
  /// and moves it to the top group (setWorkspacePinned keeps the invariant).
  const togglePinFromMenu = () => {
    const id = menu?.workspaceId
    setMenu(null)
    if (id == null) return
    const ws = stateRef.current.workspaces.find((w) => w.id === id)
    persist(setWorkspacePinned(stateRef.current, id, ws?.pinned !== true))
  }

  /// Rename from the context menu (#37): opens the SAME inline edit the pencil
  /// icon uses — one rename path everywhere.
  const renameFromMenu = () => {
    const id = menu?.workspaceId
    setMenu(null)
    if (id == null) return
    const ws = stateRef.current.workspaces.find((w) => w.id === id)
    if (ws != null) startRename(ws)
  }

  const minimize = () => { void getCurrentWindow().minimize() }
  const toggleMaximize = () => { void getCurrentWindow().toggleMaximize() }
  const close = () => { void getCurrentWindow().close() }

  // Split a workspace's ACTIVE panel (v0.2 / #25 — unlimited panels, no cap;
  // the split targets whichever panel is focused). The layout tree is
  // persisted with the workspace, so the split survives a restart; the new
  // leaf inherits the target's panel config (#29) and a freshness snapshot
  // rides the same save.
  const splitWorkspace = (id: string, orientation: Orientation) => {
    persist(splitPanel(state, id, orientation))
    void snapshotAndPersist()
  }

  // Focus a panel of a workspace — wired to BOTH pane clicks and keystrokes
  // (HITL round 6: typing in a panel makes it the active one). Only re-renders
  // when the active panel actually changes, so per-keystroke calls on an
  // already-active panel are free.
  //
  // MUST read stateRef.current, not the closure `state`: TerminalSurface
  // registers its onData handler once at mount, so every keystroke arrives
  // through a first-render closure. Reading the captured `state` here
  // replaced the WHOLE state with a stale snapshot on every keypress — the
  // app visibly jumped back to another workspace while typing (HITL: "press
  // d and it switches workspaces").
  const focusWorkspacePanel = (id: string, panelId: string) => {
    applySignal(panelId, 'focus')
    if (activePanelOf(stateRef.current, id) !== panelId) {
      setState(focusPanel(stateRef.current, id, panelId))
    }
  }

  // --- Safe panel closing (v0.2 Phase 4 / #28) -----------------------------
  //
  // ONE rule for every close path (X button, Ctrl+Shift+W, workspace close):
  // ask the backend whether a live process owns the panel's terminal; if yes,
  // confirm through the shared dialog naming the risk; if no (idle prompt or
  // an already-exited shell), close immediately and silently. The panelId →
  // backend-handle map is filled by each surface's onOpened report.
  type PtyEntry = { kind: 'local' | 'ssh'; id: number }
  const ptyIdsRef = useRef<Map<string, PtyEntry>>(new Map())
  const notePanelOpened = useCallback(
    (panelId: string, ptyId: number, remote: boolean) => {
      ptyIdsRef.current.set(panelId, { kind: remote ? 'ssh' : 'local', id: ptyId })
    },
    [],
  )

  // A close waiting on the user's confirmation (null = none pending).
  // Rendered by CloseConfirmDialog.
  type PendingClose =
    | { kind: 'panel'; workspaceId: string; panelId: string; label: string }
    | { kind: 'tab'; workspaceId: string; tabId: string; label: string; busyCount: number }
    | { kind: 'workspace'; workspaceId: string; busyCount: number }
    | null
  const [pendingClose, setPendingClose] = useState<PendingClose>(null)

  /// Perform the actual per-panel close (post-confirmation or idle shortcut).
  const doClosePanel = (id: string, panelId: string) => {
    persist(closePanel(stateRef.current, id, panelId))
    // Freshness snapshot (#29): survivors' cwds land in the same save the
    // layout change writes; the closed leaf's entry was dropped above.
    void snapshotAndPersist()
  }

  /// Ask-or-close for one panel. Remote panels skip the check: the remote
  /// side is opaque by policy (OSC-only, no polling), and AC2 demands idle
  /// panels never ask.
  const requestClosePanel = (id: string, panelId: string) => {
    const entry = ptyIdsRef.current.get(panelId)
    if (entry == null || entry.kind !== 'local') {
      doClosePanel(id, panelId)
      return
    }
    void invoke<boolean>('pty_is_busy', { id: entry.id })
      .then((busy) => {
        if (!busy) {
          doClosePanel(id, panelId)
          return
        }
        const ws = stateRef.current.workspaces.find((w) => w.id === id)
        const short = panelId.slice(0, 4)
        setPendingClose({
          kind: 'panel',
          workspaceId: id,
          panelId,
          label: `${ws?.name ?? 'workspace'} · ${short}`,
        })
      })
      .catch((e) => {
        // Unknown handle (shell already gone) or a failed check: fall back to
        // v0.1's silent close rather than trapping the panel in the UI.
        console.error('pty_is_busy failed:', e)
        doClosePanel(id, panelId)
      })
  }

  /// Ask-or-close for a whole workspace: one confirmation naming HOW MANY
  /// panels have live processes, when any of them does. With no local panels
  /// to check (e.g. all remote, or none mounted yet) nothing can be busy, so
  /// the close happens synchronously like v0.1.
  const requestCloseWorkspace = (id: string) => {
    const locals = panelIdsOf(state, id)
      .map((pid) => ptyIdsRef.current.get(pid))
      .filter((e): e is PtyEntry => e != null && e.kind === 'local')
    if (locals.length === 0) {
      setState(closeWorkspace(stateRef.current, id))
      void snapshotAndPersist()
      return
    }
    void Promise.all(
      locals.map((e) =>
        invoke<boolean>('pty_is_busy', { id: e.id }).catch(() => false),
      ),
    ).then((results) => {
      const busyCount = results.filter(Boolean).length
      if (busyCount === 0) {
        setState(closeWorkspace(stateRef.current, id))
        void snapshotAndPersist()
        return
      }
      setPendingClose({ kind: 'workspace', workspaceId: id, busyCount })
    })
  }

  const confirmPendingClose = () => {
    const pc = pendingClose
    setPendingClose(null)
    if (pc == null) return
    if (pc.kind === 'panel') doClosePanel(pc.workspaceId, pc.panelId)
    else if (pc.kind === 'tab') {
      persist(closeTab(stateRef.current, pc.workspaceId, pc.tabId))
      void snapshotAndPersist()
    } else {
      setState(closeWorkspace(stateRef.current, pc.workspaceId))
      void snapshotAndPersist()
    }
  }

  /// Ask-or-close for a TAB (#37 rework): one confirmation naming HOW MANY of
  /// the tab's panels have live processes, when any of them does — the same
  /// rule the workspace close uses (#28), one level down. With no local
  /// panels to check nothing can be busy, so the close is synchronous.
  const requestCloseTab = (workspaceId: string, tabId: string, label: string) => {
    const ws = stateRef.current.workspaces.find((w) => w.id === workspaceId)
    const tab = ws?.tabs?.find((t) => t.id === tabId)
    if (ws == null || tab == null) return
    const locals = leafIds(tab.layout)
      .map((pid) => ptyIdsRef.current.get(pid))
      .filter((e): e is PtyEntry => e != null && e.kind === 'local')
    if (locals.length === 0) {
      persist(closeTab(stateRef.current, workspaceId, tabId))
      void snapshotAndPersist()
      return
    }
    void Promise.all(
      locals.map((e) =>
        invoke<boolean>('pty_is_busy', { id: e.id }).catch(() => false),
      ),
    ).then((results) => {
      const busyCount = results.filter(Boolean).length
      if (busyCount === 0) {
        persist(closeTab(stateRef.current, workspaceId, tabId))
        void snapshotAndPersist()
        return
      }
      setPendingClose({ kind: 'tab', workspaceId, tabId, label, busyCount })
    })
  }

  // --- Delete-workspace confirmation ----------------------------------------
  //
  // window.confirm is a dead end in the macOS WebView (WKWebView): it never
  // shows a dialog and synchronously returns false, so the delete silently
  // did nothing (HITL: "the Delete Workspace button doesn't work"). We render
  // the SAME shared modal the close paths use instead — one confirmation
  // language everywhere.
  const [pendingDelete, setPendingDelete] = useState<{ id: string; name: string } | null>(
    null,
  )
  const confirmPendingDelete = () => {
    const pd = pendingDelete
    setPendingDelete(null)
    if (pd == null) return
    persist(deleteWorkspace(stateRef.current, pd.id))
  }

  // Dragging a divider (story 18) updates state live (cheap, in-memory) while
  // the final ratio is persisted once, on pointer-up — one config write per
  // drag, not one per pointer-move. stateRef always holds the latest state.
  const stateRef = useRef(state)
  stateRef.current = state

  // --- Per-panel agent status (v0.2 Phase 2 / #26) -------------------------
  //
  // One pure AgentStatusMachine per panel leaf, held OUTSIDE React state:
  // output chunks fire onActivity many times per second and must not re-render
  // per chunk — only actual status TRANSITIONS update `statuses`. The clock is
  // injected (performance.now); the machine itself is unit-tested in
  // agentStatus.test.ts. This block is glue, verified in the HITL pass.
  const machinesRef = useRef<Map<string, AgentStatusMachine>>(new Map())
  // Counts 500ms ticks so the presence poll fires every 4th one (~2s).
  const presenceTickCount = useRef(0)
  const [statuses, setStatuses] = useState<Record<string, AgentStatus>>({})
  const applySignal = useCallback(
    (
      panelId: string,
      signal: 'activity' | 'completion' | 'focus' | 'resize' | 'input' | 'presence',
      value?: number | boolean,
    ) => {
      const machines = machinesRef.current
      let machine = machines.get(panelId)
      if (machine == null) {
        machine = new AgentStatusMachine()
        machines.set(panelId, machine)
      }
      const before = machine.status
      const now = performance.now()
      if (signal === 'activity') machine.onActivity(now, value as number)
      else if (signal === 'completion') machine.onCompletion(now)
      else if (signal === 'resize') machine.onRedraw(now)
      else if (signal === 'input') machine.onUserInput(now, value === true)
      else if (signal === 'presence') machine.onPresence(now, value === true)
      else machine.onFocus(now)
      const after = machine.status
      if (after !== before) {
        setStatuses((prev) => ({ ...prev, [panelId]: after }))
      }
    },
    [],
  )
  const notePanelActivity = useCallback(
    (panelId: string, bytes: number) => applySignal(panelId, 'activity', bytes),
    [applySignal],
  )
  const notePanelCompletion = useCallback(
    (panelId: string) => applySignal(panelId, 'completion'),
    [applySignal],
  )
  const notePanelViewportResize = useCallback(
    (panelId: string) => applySignal(panelId, 'resize'),
    [applySignal],
  )
  const notePanelUserInput = useCallback(
    (panelId: string, submitted: boolean) => applySignal(panelId, 'input', submitted),
    [applySignal],
  )

  // Revealing a workspace makes its terminals repaint (geometry + focus
  // reporting), and hiding one can too — neither is agent work. On every
  // active-workspace switch, arm the machines' redraw-suppression window for
  // every open workspace's panels so the switch never flickers a Running
  // flash (HITL #1). Real streaming outlasts the 1s window and still shows.
  useEffect(() => {
    if (state.activeId == null) return
    for (const ws of stateRef.current.workspaces) {
      if (!stateRef.current.openIds.includes(ws.id)) continue
      for (const pid of panelIdsOf(stateRef.current, ws.id)) {
        applySignal(pid, 'resize')
      }
    }
  }, [state.activeId, applySignal])

  // Quiet-check tick (working -> idle once output goes silent) plus pruning of
  // machines whose panel no longer exists in any open workspace (closing a
  // panel or workspace must not leak its machine). One interval for every
  // panel; 500ms is well under the machine's 2s quiet window.
  useEffect(() => {
    const timer = window.setInterval(() => {
      const machines = machinesRef.current
      const live = new Set<string>()
      for (const ws of stateRef.current.workspaces) {
        if (!stateRef.current.openIds.includes(ws.id)) continue
        for (const pid of panelIdsOf(stateRef.current, ws.id)) live.add(pid)
      }
      // Prune backend-handle reports alongside the machines (#28): a closed
      // panel's entry must go so the map only ever describes live panels.
      for (const pid of ptyIdsRef.current.keys()) {
        if (!live.has(pid)) ptyIdsRef.current.delete(pid)
      }
      let changed = false
      const now = performance.now()
      for (const [panelId, machine] of machines) {
        if (!live.has(panelId)) {
          machines.delete(panelId)
          changed = true
          continue
        }
        const before = machine.status
        machine.onTick(now)
        if (machine.status !== before) changed = true
      }
      if (changed) {
        const snapshot: Record<string, AgentStatus> = {}
        for (const [panelId, machine] of machines) snapshot[panelId] = machine.status
        setStatuses(snapshot)
      }

      // Agent-status presence (model v2 / HITL 2026-08-25): every 4th tick
      // (~2s) ask the backend for each LOCAL panel's foreground program name
      // and feed the machines a known-CLI presence signal. The backend does
      // a process-table lookup (never reads terminal content); remote panels
      // are skipped — the local foreground is always the ssh client there.
      presenceTickCount.current += 1
      if (presenceTickCount.current % 4 === 0) {
        const localPanels = [...ptyIdsRef.current.entries()]
          .filter(([, entry]) => entry.kind === 'local')
          .map(([panelId, entry]) => ({ panelId, ptyId: entry.id }))
        if (localPanels.length > 0) {
          void invoke<{ panelId: string; process: string | null }[]>(
            'panel_processes',
            { panels: localPanels },
          )
            .then((answers) => {
              for (const answer of answers) {
                applySignal(answer.panelId, 'presence', isAiCliProcess(answer.process))
              }
            })
            .catch((e) => console.error('panel_processes failed:', e))
        }
      }
    }, 500)
    return () => window.clearInterval(timer)
  }, [applySignal])
  const resizeWorkspacePanel = (
    id: string,
    splitId: string,
    ratio: number,
    container: Container,
  ) => {
    setState(resizePanel(state, id, splitId, ratio, container))
  }
  const commitResize = () => {
    persist(stateRef.current)
  }

  return (
    // is-sidebar-collapsed (#39 follow-up): carried on the shell so the tab
    // bar can reserve room for the expand toggle seated before the tabs.
    <div className={collapsed ? 'shell is-sidebar-collapsed' : 'shell'}>
      {collapsed && (
        <button
          className="sidebar-expand"
          aria-label="Expand sidebar"
          title="Expand sidebar"
          onClick={() => setCollapsed(false)}
        >
          <SidebarExpandIcon />
        </button>
      )}
      {/* #39: the sidebar is ALWAYS mounted — collapsing animates its width to
          zero (is-collapsed) instead of unmounting, so the slide is possible. */}
      <aside
        className={collapsed ? 'sidebar is-collapsed' : 'sidebar'}
        onContextMenu={(e) => openMenu(e, false)}
        onMouseDown={(e) => {
          if (isMenuPress(e)) openMenu(e, false)
        }}
      >
        {/* Fixed-width inner column (#39): the sidebar animates its width;
            this wrapper holds 240px so contents never reflow mid-slide. */}
        <div className="sidebar-inner">
          <div
            className="sidebar-header"
            onContextMenu={(e) => openMenu(e, true)}
            onMouseDown={(e) => {
              if (isMenuPress(e)) openMenu(e, true)
            }}
          >
            <div className="wordmark">
              <span>umux</span>
            </div>
            <div className="header-actions">
              <NotificationMuteButton muted={muted} onToggle={toggleMute} />
              <button
                className="icon-btn"
                aria-label="Settings"
                title="Settings"
                onClick={() => setSettingsOpen(true)}
              >
                <SettingsIcon />
              </button>
              <button
                className="icon-btn"
                aria-label="New workspace"
                title="New workspace"
                onClick={startCreate}
              >
                <PlusIcon />
              </button>
              <button
                className="icon-btn"
                aria-label="Collapse sidebar"
                title="Collapse sidebar"
                onClick={() => setCollapsed(true)}
              >
                <SidebarCollapseIcon />
              </button>
            </div>
          </div>

        {creating && (
          <div className="create-form">
            <input
              className="text-input"
              aria-label="New workspace name"
              autoFocus
              value={draftName}
              placeholder="workspace name"
              onChange={(e) => setDraftName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleCreate()
                if (e.key === 'Escape') cancelCreate()
              }}
            />
            <button className="btn-primary" onClick={handleCreate}>
              Create
            </button>
            {/* #37: a visible way out of the creation form — same path as
                Escape, for when the keyboard is not the instinct. */}
            <button
              className="icon-btn"
              aria-label="Cancel creating workspace"
              title="Cancel"
              onClick={cancelCreate}
            >
              <CloseIcon />
            </button>
          </div>
        )}

        <ul className="workspace-list">
          {state.workspaces.map((ws) => (
            <li
              key={ws.id}
              data-testid={`workspace-row-${ws.id}`}
              className={`workspace-row ${ws.id === state.activeId ? 'is-active' : ''}`}
              draggable
              onDragStart={() => setDraggedId(ws.id)}
              onDragOver={(e) => e.preventDefault()}
              onDrop={(e) => {
                e.preventDefault()
                if (draggedId == null || draggedId === ws.id) return
                const targetIndex = state.workspaces.findIndex(
                  (w) => w.id === ws.id,
                )
                persist(moveWorkspace(state, draggedId, targetIndex))
                setDraggedId(null)
              }}
              onClick={() => setState(openWorkspace(state, ws.id))}
              onContextMenu={(e) => openMenu(e, false, ws.id)}
              onMouseDown={(e) => {
                if (isMenuPress(e)) openMenu(e, false, ws.id)
              }}
            >
              {editingId === ws.id ? (
                <input
                  className="text-input"
                  aria-label="Rename workspace"
                  autoFocus
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  onClick={(e) => e.stopPropagation()}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') commitRename(ws.id)
                    if (e.key === 'Escape') setEditingId(null)
                  }}
                />
              ) : (
                <>
                  {/* #37: pinned workspaces carry a pin glyph; the pinned
                      group always leads the list (setWorkspacePinned). */}
                  {ws.pinned === true && <PinIcon className="row-pin" />}
                  <span className="workspace-name">{ws.name}</span>
                  {settings.agentStatusEnabled &&
                    state.openIds.includes(ws.id) &&
                    (() => {
                      // One chip per panel, the ACTIVE panel's chip first with
                      // the bigger dot (HITL round 5: the big dot marks the
                      // terminal you're in — switching panel focus moves it),
                      // the remaining panels' statuses as minis below. Since
                      // the #37 rework this covers EVERY tab's panes — all of
                      // a workspace's statuses live here, on its row (Adam's
                      // call: tabs stay clean, no chips on the tab bar).
                      const pids = panelIdsOf(state, ws.id)
                      if (pids.length === 0) return null
                      const activePid = activePanelOf(state, ws.id) ?? pids[0]
                      // Fixed chip order (panel tree order) — chips never move
                      // when focus changes; ONLY the active panel's dot grows
                      // (mini -> full), exactly where that panel's chip sits
                      // (HITL round 7).
                      return (
                        <span className="workspace-statuses">
                          {pids.map((pid) => (
                            <AgentStatusIndicator
                              key={pid}
                              status={statuses[pid] ?? 'idle'}
                              mini={pid !== activePid}
                            />
                          ))}
                        </span>
                      )
                    })()}
                  <button
                    className="icon-btn"
                    aria-label={`Rename ${ws.name}`}
                    title="Rename"
                    onClick={(e) => {
                      e.stopPropagation()
                      startRename(ws)
                    }}
                  >
                    <PencilIcon />
                  </button>
                  <button
                    className="icon-btn"
                    aria-label={`Close ${ws.name}`}
                    title="Close (keep workspace)"
                    onClick={(e) => {
                      e.stopPropagation()
                      // v0.2 Phase 4 / #28: workspace close asks when ANY of
                      // its panels has a live process (shared dialog).
                      requestCloseWorkspace(ws.id)
                    }}
                  >
                    <CloseIcon />
                  </button>
                </>
              )}
            </li>
          ))}
          {state.workspaces.length === 0 && (
            <li className="empty-hint">No workspaces yet.</li>
          )}
        </ul>
        </div>
      </aside>

      <main className="main">
        {fallbackMessage && (
          <div className="fallback-banner" role="alert">
            <span>{fallbackMessage}</span>
            <button
              className="fallback-dismiss"
              aria-label="Dismiss warning"
              title="Dismiss"
              onClick={() => setFallbackMessage(null)}
            >
              <CloseIcon />
            </button>
          </div>
        )}
        {/* `.panels` is the positioning context for the absolute `.panel`s
            (that used to be `.main` itself). */}
        <div className="panels">
          {state.workspaces.length === 0 && !creating ? (
            <EmptyState onCreate={startCreate} />
          ) : null}
          {state.workspaces
          .filter((ws) => state.openIds.includes(ws.id))
          .map((ws) => {
            const isActive = ws.id === state.activeId
            const activeTab = activeTabOf(state, ws.id)
            return (
              <div
                key={ws.id}
                data-testid={`panel-${ws.id}`}
                className={`panel ${isActive ? '' : 'is-hidden'}`}
                data-split-orientation={
                  activeTab?.layout.kind === 'split'
                    ? activeTab.layout.orientation
                    : undefined
                }
              >
                {/* Terminal tabs (#37 rework, Adam's correction): tabs are
                    the separate terminal WINDOWS of THIS workspace — a
                    workspace is the project, tabs are the terminals open in
                    it, and a tab's area splits into panes. Each workspace
                    renders its own bar; switching tabs keeps every shell
                    mounted (hidden panes below). No status chips here —
                    agent statuses stay on the workspace rows in the
                    sidebar, covering every tab's panes. */}
                <div
                  className="tab-bar"
                  role="tablist"
                  aria-label={`${ws.name} terminals`}
                >
                  {(ws.tabs ?? []).map((tab, i) => {
                    const tabActive = tab.id === activeTab?.id
                    const only = (ws.tabs ?? []).length <= 1
                    const editing = editingTab?.wsId === ws.id && editingTab.tabId === tab.id
                    const label = tab.name ?? `Tab ${i + 1}`
                    return (
                      <div
                        key={tab.id}
                        role="tab"
                        aria-selected={tabActive}
                        data-testid={`tab-${ws.id}-${tab.id}`}
                        className={`tab ${tabActive ? 'is-active' : ''}`}
                        title={`${ws.name} · ${label}`}
                        onClick={() => setState(switchTab(state, ws.id, tab.id))}
                        onDoubleClick={(e) => {
                          e.stopPropagation()
                          startTabRename(ws.id, tab.id, tab.name)
                        }}
                        onContextMenu={(e) => openMenu(e, false, ws.id, tab.id)}
                        onMouseDown={(e) => {
                          // A right-press must open the menu WITHOUT letting
                          // WebKit start a text selection of the tab title
                          // (HITL: right-click highlighted the label).
                          if (isMenuPress(e)) {
                            e.preventDefault()
                            setState(switchTab(state, ws.id, tab.id))
                            openMenu(e, false, ws.id, tab.id)
                          }
                        }}
                      >
                        {tab.pinned === true && <PinIcon className="tab-pin" />}
                        {editing ? (
                          <input
                            className="tab-rename"
                            aria-label="Rename tab"
                            autoFocus
                            value={editTabName}
                            onChange={(e) => setEditTabName(e.target.value)}
                            // The current name is visible AND pre-selected —
                            // typing replaces it outright, arrows/Escape edit
                            // it in place.
                            onFocus={(e) => e.currentTarget.select()}
                            onClick={(e) => e.stopPropagation()}
                            onDoubleClick={(e) => e.stopPropagation()}
                            onKeyDown={(e) => {
                              if (e.key === 'Enter') commitTabRename(ws.id, tab.id)
                              if (e.key === 'Escape') cancelTabRename()
                            }}
                            onBlur={() => commitTabRename(ws.id, tab.id)}
                          />
                        ) : (
                          <span className="tab-name">{label}</span>
                        )}
                        <button
                          className="tab-close"
                          aria-label={`Close ${label}`}
                          title={
                            only
                              ? 'A workspace keeps at least one terminal'
                              : 'Close tab'
                          }
                          disabled={only}
                          onClick={(e) => {
                            e.stopPropagation()
                            requestCloseTab(ws.id, tab.id, tabMenuLabel(ws, tab.id, i))
                          }}
                        >
                          <CloseIcon />
                        </button>
                      </div>
                    )
                  })}
                  <button
                    className="tab-add icon-btn"
                    aria-label="New terminal tab"
                    title="New terminal tab"
                    onClick={() => persist(addTab(stateRef.current, ws.id))}
                  >
                    <PlusIcon />
                  </button>
                </div>
                {/* Every tab's panes stay mounted (hidden when not active) so
                    each keeps its shells — the same contract workspace
                    switching already follows. */}
                <div className="tab-panes-area">
                  {(ws.tabs ?? []).map((tab) => (
                    <div
                      key={tab.id}
                      data-testid={`tab-panes-${tab.id}`}
                      className={`tab-panes ${tab.id === activeTab?.id ? '' : 'is-hidden'}`}
                    >
                      <PanelSurfaces
                        workspaceId={ws.id}
                        workspaceName={ws.name}
                        layout={tab.layout}
                        activePanelId={
                          tab.id === activeTab?.id ? activePanelOf(state, ws.id) : null
                        }
                        firstLeafId={leafIds(tab.layout)[0] ?? ''}
                        // Session-restore toggle off (#27 HITL follow-up):
                        // strip the saved cwds so every shell starts in the
                        // default directory. sshTarget survives — a
                        // configured remote panel must not silently become
                        // local.
                        panels={
                          settings.sessionRestoreEnabled
                            ? ws.panels
                            : (ws.panels ?? []).map((p) =>
                                p.workingDirectory == null
                                  ? p
                                  : { ...p, workingDirectory: undefined },
                              )
                        }
                        onResize={(splitId, ratio, container) =>
                          resizeWorkspacePanel(ws.id, splitId, ratio, container)
                        }
                        onResizeEnd={commitResize}
                        onClose={(panelId) => requestClosePanel(ws.id, panelId)}
                        onFocusPanel={(panelId) => focusWorkspacePanel(ws.id, panelId)}
                        onPanelActivity={notePanelActivity}
                        onPanelCompletion={notePanelCompletion}
                        onPanelViewportResize={notePanelViewportResize}
                        onPanelUserInput={(panelId, submitted) => {
                          focusWorkspacePanel(ws.id, panelId)
                          notePanelUserInput(panelId, submitted)
                        }}
                        onPanelOpened={notePanelOpened}
                        statuses={statuses}
                        statusEnabled={settings.agentStatusEnabled}
                      />
                    </div>
                  ))}
                </div>
              </div>
            )
          })}
        </div>
      </main>

      {settingsOpen && (
        <SettingsDialog
          settings={settings}
          onChange={applySettings}
          onClose={() => setSettingsOpen(false)}
        />
      )}

      {pendingClose != null && (
        <CloseConfirmDialog
          title={
            pendingClose.kind === 'panel'
              ? 'Close this panel?'
              : pendingClose.kind === 'tab'
                ? 'Close this tab?'
                : 'Close this workspace?'
          }
          message={
            pendingClose.kind === 'panel'
              ? `Panel ${pendingClose.label} has a running process. Closing it now will terminate that process.`
              : pendingClose.kind === 'tab'
                ? `${pendingClose.busyCount} panel${pendingClose.busyCount === 1 ? '' : 's'} in tab ${pendingClose.label} ha${pendingClose.busyCount === 1 ? 's' : 've'} a running process. Closing it now will terminate ${pendingClose.busyCount === 1 ? 'it' : 'them'}.`
                : `${pendingClose.busyCount} panel${pendingClose.busyCount === 1 ? '' : 's'} in this workspace ha${pendingClose.busyCount === 1 ? 's' : 've'} a running process. Closing it now will terminate ${pendingClose.busyCount === 1 ? 'it' : 'them'}.`
          }
          confirmLabel="Close anyway"
          onConfirm={confirmPendingClose}
          onCancel={() => setPendingClose(null)}
        />
      )}

      {pendingDelete != null && (
        <CloseConfirmDialog
          title="Delete this workspace?"
          message={`Delete "${pendingDelete.name}" and close its panels? Its saved layout and settings for these panels are removed. This cannot be undone.`}
          confirmLabel="Delete"
          onConfirm={confirmPendingDelete}
          onCancel={() => setPendingDelete(null)}
        />
      )}

      {menu && (
        <div
          className="context-menu"
          style={{ left: menu.x, top: menu.y }}
          role="menu"
          // Any click inside the menu (item or padding) closes it — the
          // window-level close listener alone is not enough now that it is
          // time-guarded against the opening gesture.
          onClick={() => setMenu(null)}
        >
          {menu.tabId == null && (
            <button className="menu-item" role="menuitem" onClick={startCreate}>
              <PlusIcon />
              New workspace
            </button>
          )}
          {/* Tab-scoped actions (#37 follow-up): the menu opened from a
              TERMINAL TAB offers the tab's own actions — rename, pin, close —
              plus the splits (they target the tab's active panel). The
              workspace-list actions below stay hidden here. */}
          {menu.tabId != null &&
            menu.workspaceId != null &&
            (() => {
              const ws = state.workspaces.find((w) => w.id === menu.workspaceId)
              const index = ws?.tabs?.findIndex((t) => t.id === menu.tabId) ?? -1
              const tab = ws?.tabs?.[index]
              const pinned = tab?.pinned === true
              const only = (ws?.tabs ?? []).length <= 1
              return (
                <>
                  <button
                    className="menu-item"
                    role="menuitem"
                    onClick={renameTabFromMenu}
                  >
                    <PencilIcon />
                    Rename tab
                  </button>
                  <button
                    className="menu-item"
                    role="menuitem"
                    onClick={togglePinTabFromMenu}
                  >
                    <PinIcon />
                    {pinned ? 'Unpin tab' : 'Pin tab'}
                  </button>
                  <div className="menu-separator" />
                  <button
                    className="menu-item"
                    role="menuitem"
                    onClick={() => splitFromMenu('horizontal')}
                  >
                    <SplitHorizontalIcon />
                    Split horizontal
                  </button>
                  <button
                    className="menu-item"
                    role="menuitem"
                    onClick={() => splitFromMenu('vertical')}
                  >
                    <SplitVerticalIcon />
                    Split vertical
                  </button>
                  <div className="menu-separator" />
                  <button
                    className="menu-item danger"
                    role="menuitem"
                    disabled={only}
                    title={only ? 'A workspace keeps at least one terminal' : undefined}
                    onClick={closeTabFromMenu}
                  >
                    <CloseIcon />
                    Close tab
                  </button>
                </>
              )
            })()}
          {menu.tabId == null && menu.workspaceId && (() => {
            // v0.2 / #25: unlimited panels — the split actions are always
            // available (they split the workspace's active panel).
            // #37: Pin and Rename sit between the splits and Delete (Adam's
            // requested position: Pin directly before Delete).
            const menuTarget = state.workspaces.find(
              (w) => w.id === menu.workspaceId,
            )
            const pinned = menuTarget?.pinned === true
            return (
              <>
                <div className="menu-separator" />
                <button
                  className="menu-item"
                  role="menuitem"
                  onClick={() => splitFromMenu('horizontal')}
                >
                  <SplitHorizontalIcon />
                  Split horizontal
                </button>
                <button
                  className="menu-item"
                  role="menuitem"
                  onClick={() => splitFromMenu('vertical')}
                >
                  <SplitVerticalIcon />
                  Split vertical
                </button>
                <div className="menu-separator" />
                <button
                  className="menu-item"
                  role="menuitem"
                  onClick={togglePinFromMenu}
                >
                  <PinIcon />
                  {pinned ? 'Unpin workspace' : 'Pin workspace'}
                </button>
                <button
                  className="menu-item"
                  role="menuitem"
                  onClick={renameFromMenu}
                >
                  <PencilIcon />
                  Rename workspace
                </button>
                <div className="menu-separator" />
                <button
                  className="menu-item danger"
                  role="menuitem"
                  onClick={deleteFromMenu}
                >
                  <CloseIcon />
                  Delete workspace
                </button>
              </>
            )
          })()}
          {menu.header && (
            <>
              <div className="menu-separator" />
              <button className="menu-item" role="menuitem" onClick={minimize}>
                <MinimizeIcon />
                Minimize
              </button>
              <button className="menu-item" role="menuitem" onClick={toggleMaximize}>
                <MaximizeIcon />
                Maximize
              </button>
              <button className="menu-item danger" role="menuitem" onClick={close}>
                <CloseIcon />
                Close
              </button>
            </>
          )}
        </div>
      )}
    </div>
  )
}
