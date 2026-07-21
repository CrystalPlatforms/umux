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

import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { TerminalSurface } from './TerminalSurface'
import {
  emptyState,
  createWorkspace,
  renameWorkspace,
  switchWorkspace,
  type Workspace,
  type WorkspaceState,
} from './workspaces'

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

// --- Context menu state ------------------------------------------------------

type MenuState = { x: number; y: number; header: boolean } | null

export function WorkspaceShell() {
  const [state, setState] = useState<WorkspaceState>(emptyState)
  const [creating, setCreating] = useState(false)
  const [draftName, setDraftName] = useState('')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editName, setEditName] = useState('')
  const [menu, setMenu] = useState<MenuState>(null)
  const [collapsed, setCollapsed] = useState(false)

  useEffect(() => {
    void invoke<{ workspaces: Workspace[] }>('load_workspaces').then((data) => {
      const workspaces = data.workspaces ?? []
      setState({ workspaces, activeId: workspaces[0]?.id ?? null })
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

  const openMenu = (e: React.MouseEvent, header: boolean) => {
    e.preventDefault()
    e.stopPropagation()
    setMenu({ x: e.clientX, y: e.clientY, header })
  }

  const minimize = () => { void getCurrentWindow().minimize() }
  const toggleMaximize = () => { void getCurrentWindow().toggleMaximize() }
  const close = () => { void getCurrentWindow().close() }

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
              className={`workspace-row ${ws.id === state.activeId ? 'is-active' : ''}`}
              onClick={() => setState(switchWorkspace(state, ws.id))}
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
        {state.workspaces.map((ws) => (
          <div
            key={ws.id}
            data-testid={`panel-${ws.id}`}
            className={`panel ${ws.id === state.activeId ? '' : 'is-hidden'}`}
          >
            <TerminalSurface />
          </div>
        ))}
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
