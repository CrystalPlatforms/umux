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
  splitPanel,
  resizePanel,
  closePanel,
  focusPanel,
  activePanelOf,
  bootState,
  panelIdsOf,
  type Workspace,
  type WorkspaceState,
} from './workspaces'
import { boxes, type LayoutNode, type Orientation, type Container } from './PaneLayout'
import { matchShortcut } from './shortcuts'
import { NotificationMuteButton } from './NotificationMuteButton'
import { AgentStatusIndicator } from './AgentStatusIndicator'
import { AgentStatusMachine, type AgentStatus } from './agentStatus'

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

// --- Context menu state ------------------------------------------------------

type MenuState = {
  x: number
  y: number
  header: boolean
  workspaceId?: string
} | null

/// A mouse press that should open the context menu: the right button, or
/// Ctrl+click (the macOS right-click). macOS trackpads additionally deliver a
/// two-finger click as a right mousedown WITHOUT a DOM `contextmenu` event,
/// so the menu must open here too, not only on `onContextMenu`.
function isMenuPress(e: React.MouseEvent): boolean {
  return e.button === 2 || (e.ctrlKey && e.button === 0)
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
  // Identity of the tree's first leaf — the legacy SSH wiring (Phase 16 /
  // #17) passes the workspace's first configured `sshTarget` to it.
  firstLeafId: string
  sshTarget?: string
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
  onPanelUserInput: (panelId: string) => void
  // Per-panel statuses (HITL round 3: 3 splits = 3 chips). The workspace row
  // keeps its aggregate; each pane additionally shows its own dot + label.
  statuses: Record<string, AgentStatus>
}

function PanelSurfaces({ workspaceId, workspaceName, layout, activePanelId, firstLeafId, sshTarget, onResize, onResizeEnd, onClose, onFocusPanel, onPanelActivity, onPanelCompletion, onPanelViewportResize, onPanelUserInput, statuses }: PanelSurfacesProps) {
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
              sshTarget={p.id === firstLeafId ? sshTarget : undefined}
              onActivity={(bytes) => onPanelActivity(p.id, bytes)}
              onCompletion={() => onPanelCompletion(p.id)}
              onViewportResize={() => onPanelViewportResize(p.id)}
              onUserInput={() => onPanelUserInput(p.id)}
            />
            <AgentStatusIndicator status={statuses[p.id] ?? 'idle'} />
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
  const [menu, setMenu] = useState<MenuState>(null)
  // When the current menu was opened (ms epoch). The same gesture can emit a
  // click/contextmenu right AFTER the mousedown that opened the menu (Linux/
  // Windows right-click, macOS Ctrl+click) — those must not close it again.
  const menuOpenedAtRef = useRef(0)
  const [collapsed, setCollapsed] = useState(false)
  const [draggedId, setDraggedId] = useState<string | null>(null)
  // Notification mute (Phase 14 / #15). Session-only — not persisted: the flag
  // lives in the backend, seeded as audible on app start.
  const [muted, setMuted] = useState(false)
  // Config fallback warning (Phase 18 / #19, AC3). Set when the backend emits
  // `config_fallback` (corrupt/unreadable config -> defaults), so the downgrade
  // is surfaced instead of silent. Dismissible; cleared on dismiss.
  const [fallbackMessage, setFallbackMessage] = useState<string | null>(null)

  useEffect(() => {
    void invoke<{ workspaces: Workspace[] }>('load_workspaces').then((data) => {
      // bootState keeps each workspace's persisted layout tree (v0.2 / #25 —
      // the split layout round-trips through restart) and seeds a fresh single
      // leaf for pre-v0.2 configs that have none. Every defined workspace
      // starts open with its first panel focused.
      setState(bootState(data.workspaces ?? []))
    })
  }, [])

  // Seed the mute indicator from the backend flag (source of truth).
  useEffect(() => {
    void invoke<boolean>('notifications_muted').then(setMuted)
  }, [])

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

  // Toggle the app-wide mute: flip the backend flag, then mirror its returned
  // state locally so the indicator stays in sync with the panels' threads.
  const toggleMute = () => {
    const next = !muted
    void invoke<boolean>('set_notifications_muted', { muted: next }).then(setMuted)
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
      case 'new-workspace':
        persist(createWorkspace(state, defaultWorkspaceName(state)))
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
        if (panel != null) persist(closePanel(state, activeId, panel))
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

  const startRename = (ws: Workspace) => {
    setEditingId(ws.id)
    setEditName(ws.name)
  }

  const commitRename = (id: string) => {
    const name = editName.trim()
    setEditingId(null)
    if (name !== '') persist(renameWorkspace(state, id, name))
  }

  const openMenu = (e: React.MouseEvent, header: boolean, workspaceId?: string) => {
    e.preventDefault()
    e.stopPropagation()
    menuOpenedAtRef.current = Date.now()
    setMenu({ x: e.clientX, y: e.clientY, header, workspaceId })
  }

  const deleteFromMenu = () => {
    const id = menu?.workspaceId
    setMenu(null)
    if (id == null) return
    if (!window.confirm('Delete this workspace? This cannot be undone.')) return
    persist(deleteWorkspace(state, id))
  }

  const splitFromMenu = (orientation: Orientation) => {
    const id = menu?.workspaceId
    setMenu(null)
    if (id == null) return
    splitWorkspace(id, orientation)
  }

  const minimize = () => { void getCurrentWindow().minimize() }
  const toggleMaximize = () => { void getCurrentWindow().toggleMaximize() }
  const close = () => { void getCurrentWindow().close() }

  // Split a workspace's ACTIVE panel (v0.2 / #25 — unlimited panels, no cap;
  // the split targets whichever panel is focused). The layout tree is
  // persisted with the workspace, so the split survives a restart.
  const splitWorkspace = (id: string, orientation: Orientation) => {
    persist(splitPanel(state, id, orientation))
  }

  // Focus a panel of a workspace — wired to BOTH pane clicks and keystrokes
  // (HITL round 6: typing in a panel makes it the active one). Only re-renders
  // when the active panel actually changes, so per-keystroke calls on an
  // already-active panel are free.
  const focusWorkspacePanel = (id: string, panelId: string) => {
    applySignal(panelId, 'focus')
    if (activePanelOf(state, id) !== panelId) {
      setState(focusPanel(state, id, panelId))
    }
  }

  // Close one panel of a workspace (story 20): siblings fill the freed space.
  // Persisted like the split so the layout round-trips.
  const closeWorkspacePanel = (id: string, panelId: string) => {
    persist(closePanel(state, id, panelId))
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
  const [statuses, setStatuses] = useState<Record<string, AgentStatus>>({})
  const applySignal = useCallback(
    (
      panelId: string,
      signal: 'activity' | 'completion' | 'focus' | 'resize',
      bytes?: number,
    ) => {
      const machines = machinesRef.current
      let machine = machines.get(panelId)
      if (machine == null) {
        machine = new AgentStatusMachine()
        machines.set(panelId, machine)
      }
      const before = machine.status
      const now = performance.now()
      if (signal === 'activity') machine.onActivity(now, bytes)
      else if (signal === 'completion') machine.onCompletion(now)
      else if (signal === 'resize') machine.onRedraw(now)
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
    }, 500)
    return () => window.clearInterval(timer)
  }, [])
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
    <div className="shell">
      {collapsed ? (
        <button
          className="sidebar-expand"
          aria-label="Expand sidebar"
          title="Expand sidebar"
          onClick={() => setCollapsed(false)}
        >
          <SidebarExpandIcon />
        </button>
      ) : (
        <aside
          className="sidebar"
          onContextMenu={(e) => openMenu(e, false)}
          onMouseDown={(e) => {
            if (isMenuPress(e)) openMenu(e, false)
          }}
        >
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
                if (e.key === 'Escape') {
                  setCreating(false)
                  setDraftName('')
                }
              }}
            />
            <button className="btn-primary" onClick={handleCreate}>
              Create
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
                  <span className="workspace-name">{ws.name}</span>
                  {state.openIds.includes(ws.id) &&
                    (() => {
                      // One chip per panel, the ACTIVE panel's chip first with
                      // the bigger dot (HITL round 5: the big dot marks the
                      // terminal you're in — switching panel focus moves it),
                      // the remaining panels' statuses as minis below.
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
                      setState(closeWorkspace(state, ws.id))
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
        </aside>
      )}

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
        {state.workspaces.length === 0 && !creating ? (
          <EmptyState onCreate={startCreate} />
        ) : null}
        {state.workspaces
          .filter((ws) => state.openIds.includes(ws.id))
          .map((ws) => {
            const layout = ws.layout
            const isActive = ws.id === state.activeId
            const panelIds = panelIdsOf(state, ws.id)
            return (
              <div
                key={ws.id}
                data-testid={`panel-${ws.id}`}
                className={`panel ${isActive ? '' : 'is-hidden'}`}
                data-split-orientation={layout?.kind === 'split' ? layout.orientation : undefined}
              >
                {layout != null && (
                  <PanelSurfaces
                    workspaceId={ws.id}
                    workspaceName={ws.name}
                    layout={layout}
                    activePanelId={activePanelOf(state, ws.id)}
                    firstLeafId={panelIds[0] ?? ''}
                    sshTarget={ws.panels?.[0]?.sshTarget}
                    onResize={(splitId, ratio, container) =>
                      resizeWorkspacePanel(ws.id, splitId, ratio, container)
                    }
                    onResizeEnd={commitResize}
                    onClose={(panelId) => closeWorkspacePanel(ws.id, panelId)}
                    onFocusPanel={(panelId) => focusWorkspacePanel(ws.id, panelId)}
                    onPanelActivity={notePanelActivity}
                    onPanelCompletion={notePanelCompletion}
                    onPanelViewportResize={notePanelViewportResize}
                    onPanelUserInput={(panelId) => focusWorkspacePanel(ws.id, panelId)}
                    statuses={statuses}
                  />
                )}
              </div>
            )
          })}
      </main>

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
          <button className="menu-item" role="menuitem" onClick={startCreate}>
            <PlusIcon />
            New workspace
          </button>
          {menu.workspaceId && (() => {
            // v0.2 / #25: unlimited panels — the split actions are always
            // available (they split the workspace's active panel).
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
