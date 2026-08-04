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

import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { TerminalSurface } from './TerminalSurface'
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
  type Workspace,
  type WorkspaceState,
} from './workspaces'
import { createLayout, type Orientation } from './PaneLayout'
import { matchShortcut } from './shortcuts'

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

// PanelArea — one or two terminal surfaces under a single parent, so a panel
// keeps its shell across layout changes (Phase 10 / #11). Each surface is keyed
// by its stable panel id: splitting adds a keyed surface (the original keeps
// its shell), closing one drops its key (the survivor stays mounted under this
// same parent instead of remounting into a blank terminal).
//
// With two panels a draggable divider is rendered between them; the split ratio
// (in workspace state) sizes the surfaces. Pointer events drive the drag.
type PanelAreaProps = {
  id: string
  panelIds: string[]
  orientation: Orientation
  ratio: number
  activePanelId: string | null
  workspaceName: string
  onResize: (ratio: number, container: { width: number; height: number }) => void
  onClose: (panelId: string) => void
  onFocusPanel: (panelId: string) => void
}

function PanelArea({ id, panelIds, orientation, ratio, activePanelId, workspaceName, onResize, onClose, onFocusPanel }: PanelAreaProps) {
  const surfacesRef = useRef<HTMLDivElement | null>(null)
  const horizontal = orientation === 'horizontal'
  const isSplit = panelIds.length > 1

  // flex-grow shares the row/column between the two surfaces in proportion to
  // `ratio`; the fixed-width divider sits between them. A single panel just
  // takes the whole area.
  const firstFlex = isSplit ? `${ratio} 1 0%` : '1 1 100%'
  const secondFlex = `${1 - ratio} 1 0%`

  // Human-readable panel labels for desktop notifications, so the user can tell
  // which workspace/panel finished a long-running task.
  const firstLabel = `${workspaceName} · ${horizontal ? 'left' : 'top'}`
  const secondLabel = `${workspaceName} · ${horizontal ? 'right' : 'bottom'}`

  const onPointerDown = (e: React.PointerEvent) => {
    e.preventDefault()
    const container = surfacesRef.current
    if (container == null) return
    const rect = container.getBoundingClientRect()

    const onMove = (ev: PointerEvent) => {
      const next = horizontal
        ? (ev.clientX - rect.left) / rect.width
        : (ev.clientY - rect.top) / rect.height
      onResize(next, { width: rect.width, height: rect.height })
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  return (
    <div
      ref={surfacesRef}
      className={`panel-surfaces ${isSplit ? `split-${orientation}` : ''}`}
      data-testid={`panel-surfaces-${id}`}
    >
      <div
        key={panelIds[0]}
        className={`surface ${activePanelId === panelIds[0] ? 'is-active' : ''}`}
        data-panel-id={panelIds[0]}
        style={{ flex: firstFlex }}
        onClick={() => onFocusPanel(panelIds[0])}
      >
        <TerminalSurface label={firstLabel} />
        {isSplit && <PanelCloseButton onClose={() => onClose(panelIds[0])} which="first" />}
      </div>
      {isSplit && (
        <>
          <div
            className={`divider divider-${orientation}`}
            role="separator"
            aria-label="Resize panels"
            onPointerDown={onPointerDown}
          />
          <div
            key={panelIds[1]}
            className={`surface ${activePanelId === panelIds[1] ? 'is-active' : ''}`}
            data-panel-id={panelIds[1]}
            style={{ flex: secondFlex }}
            onClick={() => onFocusPanel(panelIds[1])}
          >
            <TerminalSurface label={secondLabel} />
            <PanelCloseButton onClose={() => onClose(panelIds[1])} which="second" />
          </div>
        </>
      )}
    </div>
  )
}

type PanelCloseButtonProps = { onClose: () => void; which: 'first' | 'second' }

function PanelCloseButton({ onClose, which }: PanelCloseButtonProps) {
  return (
    <button
      className="panel-close"
      aria-label={`Close panel (${which})`}
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
  const [collapsed, setCollapsed] = useState(false)
  const [draggedId, setDraggedId] = useState<string | null>(null)

  useEffect(() => {
    void invoke<{ workspaces: Workspace[] }>('load_workspaces').then((data) => {
      const workspaces = data.workspaces ?? []
      // Each open workspace starts with one fresh panel (one live shell), and
      // that seed panel is focused from the start (Phase 11 / #12). Both are
      // runtime-only and re-seeded on every reload (persistence is story 37).
      const panelIds = Object.fromEntries(
        workspaces.map((w) => [w.id, [crypto.randomUUID()]]),
      )
      setState({
        workspaces,
        activeId: workspaces[0]?.id ?? null,
        // On startup every defined workspace is open (has a live panel).
        openIds: workspaces.map((w) => w.id),
        // Panel layout is runtime-only (Phase 9): each workspace starts single.
        // Re-seeded on every reload (persistence is story 37).
        layouts: Object.fromEntries(workspaces.map((w) => [w.id, createLayout()])),
        panelIds,
        activePanelId: Object.fromEntries(
          workspaces.map((w) => [w.id, (panelIds[w.id] ?? [])[0]]),
        ),
      })
    })
  }, [])

  // Close the context menu on any click outside it.
  useEffect(() => {
    if (menu == null) return
    const close = () => setMenu(null)
    window.addEventListener('click', close)
    window.addEventListener('contextmenu', close)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('contextmenu', close)
    }
  }, [menu])

  const persist = (next: WorkspaceState) => {
    setState(next)
    void invoke('save_workspaces', { workspaces: next.workspaces })
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
        if (activeId != null) setState(splitPanel(state, activeId, 'horizontal'))
        break
      case 'split-vertical':
        if (activeId != null) setState(splitPanel(state, activeId, 'vertical'))
        break
      case 'close-panel': {
        if (activeId == null) break
        const panel = activePanelOf(state, activeId)
        if (panel != null) setState(closePanel(state, activeId, panel))
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

  // Split a workspace's area (Phase 9 / #10). Runtime-only state — not
  // persisted (story 37), so no save_workspaces here. splitPanel is a no-op
  // past two panels; the menu items are also disabled to signal the cap.
  const splitWorkspace = (id: string, orientation: Orientation) => {
    setState(splitPanel(state, id, orientation))
  }

  // Drag the divider of a split workspace (Phase 10 / #11, story 18). The
  // ratio is the pointer's position along the split axis, clamped to the
  // minimum panel size by resizePanel (story 19). Pointer events drive the
  // whole drag — capture on pointerdown, track on pointermove, release on
  // pointerup — so the divider can be dragged beyond its own thin hit area.
  //
  // The drag handler lives in <SplitPanels> (one per workspace) so its
  // container ref is scoped to a single .panel-surfaces element.
  const closeWorkspacePanel = (id: string, panelId: string) => {
    setState(closePanel(state, id, panelId))
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
        <aside className="sidebar" onContextMenu={(e) => openMenu(e, false)}>
          <div className="sidebar-header" onContextMenu={(e) => openMenu(e, true)}>
            <div className="wordmark">
              <span>umux</span>
            </div>
            <div className="header-actions">
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
        {state.workspaces
          .filter((ws) => state.openIds.includes(ws.id))
          .map((ws) => {
            const layout = state.layouts[ws.id] ?? createLayout()
            const isActive = ws.id === state.activeId
            const splitOrientation = layout.kind === 'split' ? layout.orientation : 'horizontal'
            const panelIds = state.panelIds[ws.id] ?? [crypto.randomUUID()]
            return (
              <div
                key={ws.id}
                data-testid={`panel-${ws.id}`}
                className={`panel ${isActive ? '' : 'is-hidden'}`}
                data-split-orientation={layout.kind === 'split' ? layout.orientation : undefined}
              >
                <PanelArea
                  id={ws.id}
                  panelIds={panelIds}
                  orientation={splitOrientation}
                  ratio={layout.kind === 'split' ? layout.ratio : 0.5}
                  activePanelId={activePanelOf(state, ws.id)}
                  workspaceName={ws.name}
                  onResize={(ratio, container) =>
                    setState(resizePanel(state, ws.id, ratio, container))
                  }
                  onClose={(panelId) => closeWorkspacePanel(ws.id, panelId)}
                  onFocusPanel={(panelId) =>
                    setState(focusPanel(state, ws.id, panelId))
                  }
                />
              </div>
            )
          })}
      </main>

      {menu && (
        <div
          className="context-menu"
          style={{ left: menu.x, top: menu.y }}
          role="menu"
        >
          <button className="menu-item" role="menuitem" onClick={startCreate}>
            <PlusIcon />
            New workspace
          </button>
          {menu.workspaceId && (() => {
            const isSplit =
              state.layouts[menu.workspaceId!]?.kind === 'split'
            return (
              <>
                <div className="menu-separator" />
                <button
                  className="menu-item"
                  role="menuitem"
                  disabled={isSplit}
                  onClick={() => splitFromMenu('horizontal')}
                >
                  <SplitHorizontalIcon />
                  Split horizontal
                </button>
                <button
                  className="menu-item"
                  role="menuitem"
                  disabled={isSplit}
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
