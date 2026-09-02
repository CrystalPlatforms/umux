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

import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type MouseEvent as ReactMouseEvent } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
// Issue #72: clicking a listening port opens http://localhost:{port} in the
// system browser (the clipboard copy stays).
import { openUrl } from '@tauri-apps/plugin-opener'
import { TerminalSurface } from './TerminalSurface'
import { EmptyState } from './EmptyState'
import {
  emptyState,
  createWorkspace,
  renameWorkspace,
  openWorkspace,
  deleteWorkspace,
  setWorkspacePinned,
  createGroup,
  renameGroup,
  deleteGroup,
  deleteGroupSubtree,
  isGroupEmpty,
  moveNode,
  moveNodes,
  moveToNewGroup,
  closeWorkspaces,
  deleteNodes,
  setNodesPinned,
  batchDeleteWorkspaceCount,
  isNodePinned,
  toggleCollapse,
  unpackGroup,
  setGroupPinned,
  groupSubtreeIds,
  activeAgentCount,
  flattenSidebar,
  addTab,
  closeTab,
  moveTab,
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
  defaultGenId,
  panelIdsOf,
  upsertPanelCwd,
  toggleZoom,
  zoomedPanelOf,
  setWorkspaceColor,
  setTabColor,
  setGroupColor,
  COLOR_PALETTE,
  type Group,
  type Panel,
  type Workspace,
  type WorkspaceState,
} from './workspaces'
import {
  computeSidebarDrop,
  computeTabDrop,
  type SidebarDrop,
  type SidebarRegion,
  type TabDrop,
  type TabRegion,
} from './dragInsertion'
import { boxes, leafIds, type LayoutNode, type Orientation, type Container } from './PaneLayout'
import { matchShortcut, activeTagOf } from './shortcuts'
import {
  toggleInSelection,
  clearSelection,
  isMacPlatform,
  isMultiSelectModifier,
} from './selection'
import { NotificationMuteButton } from './NotificationMuteButton'
import { AgentStatusIndicator } from './AgentStatusIndicator'
import { AgentStatusMachine, type AgentStatus } from './agentStatus'
import { isAiCliProcess } from './aiCli'
import { SettingsDialog } from './SettingsDialog'
import { CloseConfirmDialog } from './CloseConfirmDialog'
import { CmuxImportWizard } from './CmuxImportWizard'
import { coerceSettings, defaultSettings, type Settings } from './settings'
import { applyBranchAnswers, branchDirsByTab, branchQueryDirs } from './tabBranch'
import { formatPorts, localPtyIds, unionPorts } from './tabPorts'
import {
  defaultUpdaterApi,
  downloadProgressText,
  runCheck,
  runInstall,
  UpdateFlowError,
  type UpdateState,
  type UpdateResource,
} from './updater'

// --- Icons (inline SVG, no extra dependency) ---------------------------------

// Icons accept an inline `style` so color markers (#69/#70 HITL round) can
// tint the pin/folder glyphs directly — the icon BECOMES the color marker.
type IconProps = { className?: string; style?: CSSProperties }

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

// Color swatch entry (#69): a half-filled dot — the palette picker's glyph.
function ColorIcon({ className }: IconProps) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 3a9 9 0 0 1 0 18Z" fill="currentColor" stroke="none" />
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


function PinIcon({ className, style }: IconProps) {
  // Pushpin — marks a pinned workspace (#37); tints with the node's color
  // when it is set (#69/#70 HITL round: the pin IS the color marker then).
  return (
    <svg className={className} style={style} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 17v5" />
      <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1z" />
    </svg>
  )
}

function FolderIcon({ className, style }: IconProps) {
  // Folder — marks a group row (#48) and the "Move to group…" actions (#49);
  // on a colored group (#70 HITL round) it tints with the chosen color.
  return (
    <svg className={className} style={style} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
    </svg>
  )
}

function FolderFilledIcon({ className, style }: IconProps) {
  // Filled folder — the COLLAPSED group's glyph (#50, Adam's fix): the same
  // folder shape, filled, so a collapsed group reads at a glance even before
  // the `● N` badge (or the hidden children) says so. Tints like the outline
  // folder on a colored group (#70).
  return (
    <svg className={className} style={style} viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
    </svg>
  )
}

function FolderPlusIcon({ className }: IconProps) {
  // Folder with a plus — the "New group" action (#48).
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
      <path d="M12 10v6M9 13h6" />
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

function ZoomIcon({ className }: IconProps) {
  // Corners pulling apart — expand the pane to fill the tab (#40).
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M15 3h6v6" />
      <path d="M9 21H3v-6" />
      <path d="M21 3l-7 7" />
      <path d="M3 21l7-7" />
    </svg>
  )
}

function UnzoomIcon({ className }: IconProps) {
  // Corners folding back in — restore the previous layout (#40).
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 14h6v6" />
      <path d="M20 10h-6V4" />
      <path d="M14 10l7-7" />
      <path d="M3 21l7-7" />
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
  // Set when the menu was opened from a GROUP row (#48): the group actions
  // (Rename group / Delete group) show alongside the shared ones.
  groupId?: string
  // Set when the "Move to group…" picker is open for this WORKSPACE (#49):
  // the whole menu renders the picker — existing groups as buttons, plus an
  // inline field that creates a group and files the workspace into it.
  groupPickerFor?: string
  // The picker was opened from the BATCH selection menu (#53): picking a
  // group moves EVERY selected node, not the single workspace.
  batchMove?: boolean
  // Set when the COLOR swatch picker is open for this WORKSPACE (#69):
  // the whole menu renders the eight palette swatches plus the clear entry
  // (the same in-place swap the group picker uses).
  colorPickerFor?: string
} | null

/// A mouse press that should open the context menu: the right button, or
/// Ctrl+click on macOS (the macOS right-click — on Linux/Windows Ctrl+click
/// is the multi-select modifier instead, #53, so the menu gesture is gated
/// to the Mac). macOS trackpads additionally deliver a two-finger click as a
/// right mousedown WITHOUT a DOM `contextmenu` event, so the menu must open
/// here too, not only on `onContextMenu`.
function isMenuPress(e: React.MouseEvent): boolean {
  return e.button === 2 || (e.ctrlKey && e.button === 0 && isMacPlatform())
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
  // Whether THIS workspace is the active one (HITL): the focused pane of its
  // active tab pulls keyboard focus, so a switch makes the terminal typable
  // immediately. Inactive workspaces' panes never take focus.
  focused: boolean
  // Identity of the tree's first leaf — per-leaf panel config (v0.2 Phase 5 /
  // #29) with a legacy fallback: a leaf with its own entry uses it; the first
  // leaf additionally falls back to the config's first entry (the pre-v0.2
  // shape stored one entry for the workspace's only panel).
  firstLeafId: string
  panels: Panel[] | undefined
  // Zoom view state (#40 / story 48): the tab's zoomed panel id, or null when
  // the tab renders its normal layout. Pure overlay — the tree still feeds
  // `boxes`; the zoomed pane just renders full-size over the hidden rest.
  zoomedPanelId: string | null
  onToggleZoom: (panelId: string) => void
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

function PanelSurfaces({ workspaceId, workspaceName, layout, activePanelId, focused, firstLeafId, panels, zoomedPanelId, onToggleZoom, onResize, onResizeEnd, onClose, onFocusPanel, onPanelActivity, onPanelCompletion, onPanelViewportResize, onPanelUserInput, onPanelOpened, statuses, statusEnabled }: PanelSurfacesProps) {
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
        // Zoom overlay (#40): the zoomed pane renders percent-sized over the
        // whole tab; covered panes stay MOUNTED but hidden (display:none —
        // the same contract inactive tabs follow, so their shells and PTY
        // streams keep running; the reveal re-fits via ResizeObserver).
        const zoomed = zoomedPanelId === p.id
        const covered = zoomedPanelId != null && !zoomed
        const classes = [
          'surface',
          activePanelId === p.id ? 'is-active' : '',
          zoomed ? 'is-zoomed' : '',
          covered ? 'is-hidden' : '',
        ]
          .filter(Boolean)
          .join(' ')
        return (
          <div
            key={p.id}
            className={classes}
            data-panel-id={p.id}
            style={
              zoomed
                ? { left: 0, top: 0, width: '100%', height: '100%' }
                : { left: p.rect.x, top: p.rect.y, width: p.rect.width, height: p.rect.height }
            }
            onClick={() => onFocusPanel(p.id)}
          >
            <TerminalSurface
              label={`${workspaceName} · ${short}`}
              sshTarget={meta?.sshTarget}
              cwd={meta?.workingDirectory}
              // The focused pane of the active tab of the ACTIVE workspace
              // owns the keyboard (HITL): a switch focuses it instantly.
              focused={focused && activePanelId === p.id}
              onActivity={(bytes) => onPanelActivity(p.id, bytes)}
              onCompletion={() => onPanelCompletion(p.id)}
              onViewportResize={() => onPanelViewportResize(p.id)}
              onUserInput={(submitted) => onPanelUserInput(p.id, submitted)}
              onOpened={(ptyId) => onPanelOpened(p.id, ptyId, meta?.sshTarget !== undefined)}
            />
            {statusEnabled && (
              <AgentStatusIndicator status={statuses[p.id] ?? 'idle'} />
            )}
            <PanelZoomButton
              zoomed={zoomed}
              short={short}
              onToggle={() => onToggleZoom(p.id)}
            />
            <PanelCloseButton onClose={() => onClose(p.id)} short={short} />
          </div>
        )
      })}
      {zoomedPanelId == null &&
        dividers.map((d) => (
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

type PanelZoomButtonProps = { zoomed: boolean; short: string; onToggle: () => void }

/// The per-pane zoom toggle (#40 / story 48): expands THIS panel to fill the
/// tab and — zoomed — offers the reverse. Toggles the exact same state the
/// Ctrl+Shift+Z shortcut does.
function PanelZoomButton({ zoomed, short, onToggle }: PanelZoomButtonProps) {
  return (
    <button
      className="panel-close panel-zoom"
      aria-label={zoomed ? `Unzoom panel ${short}` : `Zoom panel ${short}`}
      title={zoomed ? 'Unzoom panel' : 'Zoom panel'}
      onClick={(e) => {
        e.stopPropagation()
        onToggle()
      }}
    >
      {zoomed ? <UnzoomIcon /> : <ZoomIcon />}
    </button>
  )
}

export function WorkspaceShell() {
  const [state, setState] = useState<WorkspaceState>(emptyState)
  // What the inline create form is making (#48): a workspace or a GROUP —
  // same interaction pattern (header action / context menu -> inline name
  // field -> commit on Enter), null = nothing being created.
  const [creatingKind, setCreatingKind] = useState<'workspace' | 'group' | null>(null)
  const [draftName, setDraftName] = useState('')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editName, setEditName] = useState('')
  // Group inline rename (#48): the same one-edit-at-a-time pattern the
  // workspace rename uses, keyed by group id.
  const [editingGroupId, setEditingGroupId] = useState<string | null>(null)
  const [editGroupName, setEditGroupName] = useState('')
  // The "Move to group…" picker's fresh-name field (#49).
  const [newGroupName, setNewGroupName] = useState('')
  // Header "+" dropdown (round 2, Adam): ONE button unfolds the choice of
  // what to create — a workspace or a group — instead of two sibling icons.
  const [createMenuOpen, setCreateMenuOpen] = useState(false)
  const createMenuRef = useRef<HTMLDivElement | null>(null)
  // Tab rename (#37 follow-up): which TAB is being renamed inline (double
  // click or the tab menu) and the draft name.
  const [editingTab, setEditingTab] = useState<{ wsId: string; tabId: string } | null>(null)
  const [editTabName, setEditTabName] = useState('')
  const [menu, setMenu] = useState<MenuState>(null)
  // Viewport-clamped menu position (HITL): the menu opens AT the pointer, but
  // a pointer near the window's right/bottom edge used to push half the menu
  // outside the app's bounds. After the menu mounts — in a LAYOUT effect, so
  // the fix lands before the first paint and nothing flickers — its rendered
  // size is measured and the position is pulled back inside the viewport with
  // a small margin.
  const [menuPos, setMenuPos] = useState<{ left: number; top: number } | null>(null)
  useLayoutEffect(() => {
    if (menu == null) {
      setMenuPos(null)
      return
    }
    const width = menuRef.current?.offsetWidth ?? 0
    const height = menuRef.current?.offsetHeight ?? 0
    const margin = 8
    const maxLeft = Math.max(margin, window.innerWidth - width - margin)
    const maxTop = Math.max(margin, window.innerHeight - height - margin)
    setMenuPos({
      left: Math.min(Math.max(margin, menu.x), maxLeft),
      top: Math.min(Math.max(margin, menu.y), maxTop),
    })
  }, [menu])
  // When the current menu was opened (ms epoch). The same gesture can emit a
  // click/contextmenu right AFTER the mousedown that opened the menu (Linux/
  // Windows right-click, macOS Ctrl+click) — those must not close it again.
  const menuOpenedAtRef = useRef(0)
  const [collapsed, setCollapsed] = useState(false)

  // --- Sidebar resize (HITL 2026-08-30) --------------------------------------
  // The sidebar's right edge is a drag handle: the user widens/narrows it
  // (collapse/expand stays a separate gesture). Width is RUNTIME-ONLY (like
  // collapse itself): null = the CSS default. Floor is the default width
  // (narrower never helps — that's what collapse is for), ceiling ~3/4 of
  // the window so the terminal area always keeps a working slice.
  const [sidebarWidth, setSidebarWidth] = useState<number | null>(null)
  const [resizingSidebar, setResizingSidebar] = useState(false)
  const sidebarDragRef = useRef<{ startX: number; startWidth: number } | null>(null)
  const SIDEBAR_MIN_WIDTH = 240
  const sidebarMaxWidth = () => Math.floor(window.innerWidth * 0.75)
  const onSidebarResizeStart = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    e.preventDefault()
    e.currentTarget.setPointerCapture(e.pointerId)
    sidebarDragRef.current = {
      startX: e.clientX,
      startWidth: sidebarWidth ?? SIDEBAR_MIN_WIDTH,
    }
    setResizingSidebar(true)
  }
  const onSidebarResizeMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = sidebarDragRef.current
    if (drag == null) return
    const next = drag.startWidth + (e.clientX - drag.startX)
    setSidebarWidth(Math.min(Math.max(next, SIDEBAR_MIN_WIDTH), sidebarMaxWidth()))
  }
  const onSidebarResizeEnd = (e: React.PointerEvent<HTMLDivElement>) => {
    if (sidebarDragRef.current == null) return
    sidebarDragRef.current = null
    setResizingSidebar(false)
    try {
      e.currentTarget.releasePointerCapture(e.pointerId)
    } catch {
      // The capture may already be gone (pointerup outside) — nothing to do.
    }
  }

  // Multi-select (#53): an ordered list of sidebar node ids — workspaces and
  // groups, mixed. RUNTIME-ONLY (never persisted, like the active tab); the
  // pure toggle/clear helpers live in selection.ts, the batch ACTIONS over
  // the set in workspaces.ts. The ref mirrors the list for the mount-time
  // drag/menu closures (same discipline as stateRef).
  const [selectedIds, setSelectedIds] = useState<string[]>([])
  const selectedIdsRef = useRef(selectedIds)
  selectedIdsRef.current = selectedIds

  // --- Live pointer drag & drop (round 3, Adam) -----------------------------
  //
  // HTML5 drag & drop is abandoned here on purpose: WKWebView refuses to
  // start a drag session without transfer data and Tauri's native hook
  // fights the page for drop targets — and neither can show a LIVE
  // insertion line. So rows drag via POINTER events instead: press a row,
  // move past a 4px threshold, and a line follows the pointer showing where
  // the row will land (a group's middle zone highlights instead — the row
  // will FILE into it). Release commits through the same pure ops as before
  // (moveNode / moveTab); Escape cancels. The geometry lives in
  // dragInsertion.ts (pure, unit-tested); this block is glue.
  type ActiveDrag =
    | {
        kind: 'sidebar'
        // The dragged NODE IDS (#53): one for a solo drag, the whole
        // selection when the pressed row was part of a multi-selection.
        ids: string[]
        label: string
        isGroup: boolean
        regions: SidebarRegion[]
        // The dragged nodes' forbidden landings (#51, #53): each selected
        // group contributes its whole subtree, each selected workspace
        // itself — a landing inside any of them is rejected.
        forbidden: ReadonlySet<string>
        listTop: number
        x: number
        y: number
      }
    | {
        kind: 'tab'
        wsId: string
        id: string
        label: string
        regions: TabRegion[]
        barLeft: number
        x: number
        y: number
      }
  const [drag, setDrag] = useState<ActiveDrag | null>(null)
  const [sideDrop, setSideDrop] = useState<SidebarDrop | null>(null)
  const [tabDrop, setTabDrop] = useState<TabDrop | null>(null)
  // Event-time mirrors: window pointer listeners outlive renders, so they
  // read through refs (same discipline as stateRef below).
  const dragRef = useRef<ActiveDrag | null>(null)
  // The floating ghost pill travels WITH the pointer: its position updates
  // imperatively on every move (state would freeze whenever the landing
  // decision stops changing, and re-rendering the shell per mousemove is
  // waste). React only seeds the initial transform at activation.
  const ghostRef = useRef<HTMLDivElement | null>(null)
  const sideDropRef = useRef<SidebarDrop | null>(null)
  const tabDropRef = useRef<TabDrop | null>(null)
  sideDropRef.current = sideDrop
  tabDropRef.current = tabDrop
  // The press waiting to become a drag (activation happens on first move
  // past the threshold), and whether the CURRENT press did become one —
  // the rows' onClick consumes that flag so a drop never also activates
  // the row underneath.
  const dragCandidate = useRef<
    { kind: 'sidebar' | 'tab'; wsId?: string; id: string; x: number; y: number } | null
  >(null)
  const dragActive = useRef(false)
  const suppressClickRef = useRef(false)
  const listRef = useRef<HTMLUListElement | null>(null)

  /// A press that may become a sidebar drag. Left button only (right /
  /// Ctrl+click belong to the context menu), and never from a button or the
  /// rename input — those keep their own click semantics.
  const beginSidebarDrag = (e: React.PointerEvent, id: string) => {
    if (e.button !== 0 || e.ctrlKey) return
    if ((e.target as HTMLElement).closest('button, input')) return
    suppressClickRef.current = false
    dragCandidate.current = { kind: 'sidebar', id, x: e.clientX, y: e.clientY }
  }

  /// The tab-bar twin — drags stay inside their own workspace (#45).
  const beginTabDrag = (e: React.PointerEvent, wsId: string, tabId: string) => {
    if (e.button !== 0 || e.ctrlKey) return
    if ((e.target as HTMLElement).closest('button, input')) return
    suppressClickRef.current = false
    dragCandidate.current = { kind: 'tab', wsId, id: tabId, x: e.clientX, y: e.clientY }
  }

  // One set of window-level listeners drives the whole drag lifecycle; they
  // no-op while no press is pending. Reads go through the refs above, so the
  // mount-time closure stays valid forever (same discipline as stateRef).
  useEffect(() => {
    const THRESHOLD = 4

    const clearDrag = () => {
      dragRef.current = null
      dragActive.current = false
      document.body.classList.remove('is-dragging')
      document.body.classList.remove('is-drop-invalid')
      cancelExpand()
      // The trailing click of this gesture must not activate whatever row
      // ends up under the pointer.
      suppressClickRef.current = true
      dragCandidate.current = null
      setDrag(null)
      setSideDrop(null)
      setTabDrop(null)
    }

    // --- Hover-expand + not-allowed cursor (#51) ----------------------------
    //
    // Hovering a COLLAPSED group while a drag is live expands it after a
    // short delay (cmux-style: the drop target opens itself so the row can
    // be filed deeper without a second gesture). One timer at a time,
    // re-armed whenever the hovered group changes; the rows are re-measured
    // after the expand so the live line lands on the NEW layout.
    let expandTimer: number | null = null
    let expandTarget: string | null = null
    const cancelExpand = () => {
      if (expandTimer != null) window.clearTimeout(expandTimer)
      expandTimer = null
      expandTarget = null
    }
    const scheduleExpand = (gid: string) => {
      if (expandTarget === gid) return
      cancelExpand()
      expandTarget = gid
      expandTimer = window.setTimeout(() => {
        expandTimer = null
        expandTarget = null
        persist(toggleCollapse(stateRef.current, gid))
        requestAnimationFrame(() => {
          const d = dragRef.current
          if (d?.kind !== 'sidebar') return
          d.regions = measureSidebarRegions()
          dragRef.current = { ...d }
          setDrag(dragRef.current)
        })
      }, 600)
    }

    const applySidebarDrop = (drop: SidebarDrop) => {
      sideDropRef.current = drop
      setSideDrop(drop)
      // The cursor says "not allowed" over a forbidden landing (#51).
      document.body.classList.toggle('is-drop-invalid', drop.rejected)
      // Hovering a COLLAPSED group (its middle zone) opens it after the
      // delay (#51); leaving it — or any rejected zone — cancels the arm.
      if (!drop.rejected && drop.intoGroupId != null) {
        const hovered = stateRef.current.groups.find((g) => g.id === drop.intoGroupId)
        if (hovered?.collapsed === true) scheduleExpand(hovered.id)
        else cancelExpand()
      } else {
        cancelExpand()
      }
    }

    const measureSidebarRegions = (): SidebarRegion[] => {
      const listEl = listRef.current
      return listEl
        ? [...listEl.querySelectorAll<HTMLElement>('.workspace-row')].map((el) => {
            const rect = el.getBoundingClientRect()
            const nodeId = el.dataset.nodeId ?? ''
            const isGroup = el.classList.contains('group-row')
            return {
              id: nodeId,
              kind: isGroup ? 'group' : 'workspace',
              // The row's parent (#51): a group's OWN parentId, a
              // workspace's groupId; null = top level.
              parentId: isGroup
                ? (stateRef.current.groups.find((g) => g.id === nodeId)?.parentId ??
                  null)
                : (stateRef.current.workspaces.find((w) => w.id === nodeId)?.groupId ??
                  null),
              top: rect.top,
              bottom: rect.bottom,
            }
          })
        : []
    }

    const activateSidebar = (id: string, x: number, y: number): SidebarRegion[] => {
      const listEl = listRef.current
      const regions: SidebarRegion[] = measureSidebarRegions()
      // A drag started on a row that is part of the multi-selection drags the
      // WHOLE selection (#53); anywhere else it is a solo drag. Dead ids are
      // dropped so the batch move never feeds ghosts into moveNodes.
      const known = (nodeId: string): boolean =>
        stateRef.current.groups.some((g) => g.id === nodeId) ||
        stateRef.current.workspaces.some((w) => w.id === nodeId)
      const ids = selectedIdsRef.current.includes(id)
        ? selectedIdsRef.current.filter(known)
        : [id]
      const isGroup = !stateRef.current.workspaces.some((w) => w.id === id)
      const label =
        ids.length > 1
          ? `${ids.length} items`
          : isGroup
            ? (stateRef.current.groups.find((g) => g.id === id)?.name ?? '')
            : (stateRef.current.workspaces.find((w) => w.id === id)?.name ?? '')
      // Forbidden landings (#51, #53): the union of every dragged group's
      // subtree plus each dragged workspace itself.
      const forbidden = new Set<string>()
      for (const nodeId of ids) {
        if (stateRef.current.groups.some((g) => g.id === nodeId)) {
          for (const sub of groupSubtreeIds(stateRef.current, nodeId)) forbidden.add(sub)
        } else {
          forbidden.add(nodeId)
        }
      }
      const d: ActiveDrag = {
        kind: 'sidebar',
        ids,
        label,
        isGroup: ids.length === 1 && isGroup,
        regions,
        forbidden,
        listTop: listEl?.getBoundingClientRect().top ?? 0,
        x,
        y,
      }
      dragRef.current = d
      setDrag(d)
      return regions
    }

    const activateTab = (wsId: string, id: string, x: number, y: number): TabRegion[] => {
      const barEl = document.querySelector<HTMLElement>(`[data-testid="tab-bar-${wsId}"]`)
      const regions: TabRegion[] = barEl
        ? [...barEl.querySelectorAll<HTMLElement>('.tab')].map((el) => {
            const rect = el.getBoundingClientRect()
            return { id: el.dataset.tabId ?? '', left: rect.left, right: rect.right }
          })
        : []
      const tabs = stateRef.current.workspaces.find((w) => w.id === wsId)?.tabs ?? []
      const idx = tabs.findIndex((t) => t.id === id)
      const d: ActiveDrag = {
        kind: 'tab',
        wsId,
        id,
        label: tabs[idx]?.name ?? `Tab ${idx + 1}`,
        regions,
        barLeft: barEl?.getBoundingClientRect().left ?? 0,
        x,
        y,
      }
      dragRef.current = d
      setDrag(d)
      return regions
    }

    const onPointerMove = (e: PointerEvent) => {
      const cand = dragCandidate.current
      if (cand == null) return
      // An active drag must never select text it travels over.
      if (dragActive.current) e.preventDefault()
      if (!dragActive.current) {
        // A wiggle is a click; past the threshold the drag goes live.
        if (Math.hypot(e.clientX - cand.x, e.clientY - cand.y) < THRESHOLD) return
        dragActive.current = true
        // Kill any text selection the press started and freeze selection for
        // the whole drag (WKWebView ignores pointermove preventDefault here).
        document.body.classList.add('is-dragging')
        window.getSelection()?.removeAllRanges()
        // First line position lands in THIS event — it must not wait for a
        // further move to appear.
        if (cand.kind === 'sidebar') {
          activateSidebar(cand.id, e.clientX, e.clientY)
          const seeded = dragRef.current
          if (seeded?.kind === 'sidebar') {
            applySidebarDrop(
              computeSidebarDrop(e.clientY, seeded.ids[0], seeded.regions, seeded.forbidden),
            )
          }
        } else {
          const regions = activateTab(cand.wsId as string, cand.id, e.clientX, e.clientY)
          const drop = computeTabDrop(e.clientX, regions)
          tabDropRef.current = drop
          setTabDrop(drop)
        }
        return
      }
      const d = dragRef.current
      if (d == null) return
      // The ghost pill follows the pointer continuously.
      if (ghostRef.current != null) {
        ghostRef.current.style.transform = `translate(${e.clientX + 12}px, ${e.clientY + 10}px)`
      }
      if (d.kind === 'sidebar') {
        applySidebarDrop(computeSidebarDrop(e.clientY, d.ids[0], d.regions, d.forbidden))
      } else {
        const drop = computeTabDrop(e.clientX, d.regions)
        tabDropRef.current = drop
        setTabDrop(drop)
      }
    }

    const onPointerUp = () => {
      const cand = dragCandidate.current
      if (cand == null) return
      if (!dragActive.current) {
        // A plain click: nothing to commit, nothing to suppress.
        dragCandidate.current = null
        return
      }
      const d = dragRef.current
      if (d != null) {
        if (d.kind === 'sidebar') {
          const drop = sideDropRef.current
          // A rejected landing (#51: into the dragged group's own subtree)
          // commits nothing — moveNode would refuse it anyway, but the drop
          // decision is the single source of truth for the gesture. A drag
          // started on a selected row moves the WHOLE selection (#53).
          if (drop != null && !drop.rejected) {
            const parentId = drop.intoGroupId ?? drop.parentId
            persist(
              moveNodes(stateRef.current, d.ids, {
                parentId,
                ...(drop.intoGroupId == null && drop.beforeId != null
                  ? { beforeId: drop.beforeId }
                  : {}),
              }),
            )
            setSelectedIds([])
          }
        } else {
          const drop = tabDropRef.current
          const ws = stateRef.current.workspaces.find((w) => w.id === d.wsId)
          const tabs = ws?.tabs ?? []
          const fromIndex = tabs.findIndex((t) => t.id === d.id)
          const targetIdx =
            drop == null || drop.beforeId == null
              ? tabs.length
              : tabs.findIndex((t) => t.id === drop.beforeId)
          // moveTab splices AFTER removing the tab, so a forward move aims
          // one slot earlier than the raw index of the sibling.
          const newIndex =
            targetIdx < 0
              ? tabs.length
              : fromIndex > -1 && fromIndex < targetIdx
                ? targetIdx - 1
                : targetIdx
          persist(moveTab(stateRef.current, d.wsId, d.id, newIndex))
        }
      }
      clearDrag()
    }

    const onPointerCancel = () => {
      if (dragCandidate.current != null) clearDrag()
    }

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && dragActive.current) clearDrag()
    }

    window.addEventListener('pointermove', onPointerMove)
    window.addEventListener('pointerup', onPointerUp)
    window.addEventListener('pointercancel', onPointerCancel)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('pointermove', onPointerMove)
      window.removeEventListener('pointerup', onPointerUp)
      window.removeEventListener('pointercancel', onPointerCancel)
      window.removeEventListener('keydown', onKeyDown)
      // A drag cut short by an unmount must not leave the body frozen.
      document.body.classList.remove('is-dragging')
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Escape clears the multi-selection (#53) — the same key that cancels a
  // live drag, one level up. Reads through the ref so the listener never
  // goes stale; a no-op while nothing is selected.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && selectedIdsRef.current.length > 0) {
        setSelectedIds(clearSelection())
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
  // Native WebKit menu + right-press selection suppression (HITL): with the
  // terminal's hidden textarea now holding keyboard focus (the
  // switch-to-type handoff), WKWebView answers right-clicks with its OWN
  // text-editing menu (Look Up / Copy / …) and starts a DOM selection under
  // the pointer — stacked on top of umux's context menu. Both are stopped at
  // the window in the CAPTURE phase, before any element handler runs:
  // preventDefault kills the native menu without stopping propagation, so
  // umux's React menus still open exactly as before, and a right press can
  // no longer begin a text selection. Left-drag selection — including
  // xterm's own terminal selection — is untouched.
  useEffect(() => {
    const noNativeMenu = (e: MouseEvent) => e.preventDefault()
    const noRightSelection = (e: MouseEvent) => {
      if (e.button === 2) e.preventDefault()
    }
    window.addEventListener('contextmenu', noNativeMenu, true)
    window.addEventListener('mousedown', noRightSelection, true)
    return () => {
      window.removeEventListener('contextmenu', noNativeMenu, true)
      window.removeEventListener('mousedown', noRightSelection, true)
    }
  }, [])
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

  // --- Tab branch labels (v1.0 Phase 14 / #41) ------------------------------
  //
  // PULL-only refresh (no timers, no filesystem watching): when workspace
  // STATE changes via a UI event — tab set change, focused-panel change,
  // configured directory change — recompute which directory speaks for each
  // tab and resolve that batch in ONE invoke. Answers cache by DIRECTORY
  // (two tabs on the same repo share an entry), and an unchanged directory
  // signature skips the round-trip entirely. A failed call only logs: a
  // missing label beats a broken sidebar (the row stays clean).
  const [branchLabels, setBranchLabels] = useState<Record<string, string>>({})
  const dirsSigRef = useRef('')
  useEffect(() => {
    const dirs = branchQueryDirs(state, settingsRef.current.sessionRestoreEnabled)
    // JSON (never a joined string): Windows paths may contain spaces, so any
    // single-character separator could collide two different dir sets.
    const sig = JSON.stringify(dirs)
    if (sig === dirsSigRef.current) return
    dirsSigRef.current = sig
    if (dirs.length === 0) return
    void invoke<Array<{ dir: string; branch: string | null }>>('git_branches', { dirs })
      .then((answers) => {
        // Boundary check: anything but the expected array takes the same
        // logged-skip path as a rejected invoke — never a render crash.
        if (!Array.isArray(answers)) {
          throw new Error('malformed git_branches payload')
        }
        setBranchLabels((prev) => applyBranchAnswers(prev, answers))
      })
      .catch((e) => console.error('git_branches failed:', e))
  }, [state, settings])

  // --- Listening-ports tooltip (v1.0 Phase 15 / #42; round 2 same day) -----
  //
  // PULL-only on hover of a TAB row (one tab's ports) or a WORKSPACE row in
  // the sidebar (the UNION of all its tabs' ports — one batch invoke, the
  // backend already takes many tabs). No timer, no query while nothing is
  // hovered. The result replaces the native title on tab rows — two
  // competing tooltips would be noise. The tooltip is ENTERABLE (round 2):
  // leaving the row starts a short grace close, and moving onto the tooltip
  // cancels it, so the ports stay reachable — and each port is a button that
  // copies its http://localhost URL (same navigator.clipboard path the
  // terminal copy shortcut uses). A failed/stale answer just doesn't render:
  // a tooltip is metadata, never a modal error. A row with no live LOCAL
  // shells answers instantly without a round-trip.
  const [portsTip, setPortsTip] = useState<{
    left: number
    top: number
    key: string
    ports: number[] | null
  } | null>(null)
  const portsSeq = useRef(0)
  const portsCloseTimer = useRef<number | null>(null)
  // The live context menu element, when one is rendered — the tooltip reads
  // its rect to anchor BELOW the menu (see openPortsTip).
  const menuRef = useRef<HTMLDivElement | null>(null)
  const holdPortsTip = () => {
    if (portsCloseTimer.current != null) {
      window.clearTimeout(portsCloseTimer.current)
      portsCloseTimer.current = null
    }
  }
  const openPortsTip = (
    e: ReactMouseEvent,
    key: string,
    groups: Array<{ id: string; layout: LayoutNode }>,
    // 'below' suits tab rows (bar across the top); workspace rows sit in
    // the sidebar, so their tooltip anchors at the row's TOP and juts RIGHT
    // into the terminal area at the sidebar's wall (round 3, HITL).
    side: 'below' | 'right' = 'below',
  ) => {
    // #43: the Settings switch gates the whole feature — hover does nothing
    // when it is off (no pull, no tooltip).
    if (!settings.portsTooltipEnabled) return
    holdPortsTip() // hopping straight to another row must not eat the tip
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
    const request = ++portsSeq.current
    // #43 round 2 (HITL): openMenu dismisses a live tip, but re-hovering a
    // row while the menu is still open re-arms one — and that tip may only
    // live BELOW the menu, never over its items. So while a menu is on
    // screen, the anchor moves to the menu's bottom edge.
    const menuRect = menuRef.current?.getBoundingClientRect()
    const anchor =
      menu != null && menuRect != null
        ? { left: menuRect.left, top: menuRect.bottom + 6 }
        : side === 'right'
          ? { left: rect.right + 6, top: rect.top }
          : { left: rect.left, top: rect.bottom + 6 }
    setPortsTip({ ...anchor, key, ports: null })
    const queries = groups
      .map((t) => ({ tabId: t.id, ptyIds: localPtyIds(leafIds(t.layout), ptyIdsRef.current) }))
      .filter((q) => q.ptyIds.length > 0)
    if (queries.length === 0) {
      setPortsTip((tip) => (tip && tip.key === key ? { ...tip, ports: [] } : tip))
      return
    }
    void invoke<Array<{ tabId: string; ports: number[] }>>('tab_ports', {
      tabs: queries,
    })
      .then((answers) => {
        if (request !== portsSeq.current) return // superseded / closed
        if (!Array.isArray(answers)) throw new Error('malformed tab_ports payload')
        const ports = unionPorts(
          answers.map((a) => (Array.isArray(a?.ports) ? a.ports : [])),
        )
        setPortsTip((tip) => (tip && tip.key === key ? { ...tip, ports } : tip))
      })
      .catch((err) => console.error('tab_ports failed:', err))
  }
  const closePortsTip = () => {
    // Grace window: enough time to cross the gap row→tooltip; entering the
    // tooltip cancels this, leaving the tooltip schedules it again.
    holdPortsTip()
    portsCloseTimer.current = window.setTimeout(() => {
      portsCloseTimer.current = null
      portsSeq.current += 1
      setPortsTip(null)
    }, 150)
  }
  // Issue #72: a port click now OPENS http://localhost:{port} in the system
  // browser (opener plugin) AND still copies the URL — every port opens, no
  // protocol detection (PO decision 2026-09-01). A failed open never eats
  // the copy: both paths report to the console, neither blocks the other.
  const openPort = (port: number) => {
    const url = `http://localhost:${port}`
    void openUrl(url).catch((err: unknown) =>
      console.error('open port failed:', err),
    )
    void navigator.clipboard
      ?.writeText(url)
      .catch((err) => console.error('copy port failed:', err))
  }

  // The "+" dropdown closes on any pointer press outside of it (its own
  // toggle and its two items handle themselves).
  useEffect(() => {
    if (!createMenuOpen) return
    const close = (e: PointerEvent) => {
      if (
        createMenuRef.current != null &&
        e.target instanceof Node &&
        createMenuRef.current.contains(e.target)
      ) {
        return
      }
      setCreateMenuOpen(false)
    }
    window.addEventListener('pointerdown', close)
    return () => window.removeEventListener('pointerdown', close)
  }, [createMenuOpen])

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
      invoke<{
        workspaces: Workspace[]
        groups: Group[]
        order: string[]
      }>('load_workspaces').catch(() => ({
        workspaces: [] as Workspace[],
        groups: [] as Group[],
        order: [] as string[],
      })),
      invoke<Settings>('load_settings').catch(() => defaultSettings),
    ]).then(([data, rawSettings]) => {
      setSettings(coerceSettings(rawSettings))
      // bootState keeps each workspace's persisted layout tree (v0.2 / #25 —
      // the split layout round-trips through restart) and seeds a fresh single
      // leaf for pre-v0.2 configs that have none. It also takes the sidebar
      // tree (#48): groups + the shared order; a pre-groups config carries
      // neither and loads flat. Every defined workspace starts open with its
      // first panel focused.
      setState(
        bootState(
          data.workspaces ?? [],
          defaultGenId,
          data.groups ?? [],
          data.order ?? [],
        ),
      )
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
    // #43: switching the ports tooltip OFF while one is on screen closes it
    // immediately — a live tip must not outlive its toggle.
    if (patch.portsTooltipEnabled === false) {
      portsSeq.current += 1
      holdPortsTip()
      setPortsTip(null)
    }
  }

  // The Settings footnote's settings.json mention is a LINK: clicking it
  // opens the file with the platform's default editor (backend command).
  // Fire-and-forget: a failure only logs — the dialog stays usable.
  const openSettingsFile = () => {
    void invoke('open_settings_file').catch((e) =>
      console.error('open_settings_file failed:', e),
    )
  }

  // --- App updates (issue #66) -----------------------------------------------
  // GitHub Releases is the ONLY source (zero-cost policy); the flows live in
  // updater.ts, the plugin in Rust. The startup check is deliberately QUIET:
  // offline, a release without latest.json, or an unconfigured pubkey never
  // surface at boot — only a FOUND update becomes visible (the banner). The
  // Settings row shows the same state machine, including its error states.
  const [updateState, setUpdateState] = useState<UpdateState>({ kind: 'idle' })
  // The plugin's Update handle survives between check and install; null means
  // nothing was found yet (and keeps the banner hidden even in error states).
  const [updateResource, setUpdateResource] = useState<UpdateResource | null>(null)
  const [updateBannerDismissed, setUpdateBannerDismissed] = useState(false)

  const settleCheck = (result: { state: UpdateState; update: UpdateResource | null }) => {
    setUpdateResource(result.update)
    setUpdateState(result.state)
  }

  const checkForUpdates = () => {
    setUpdateState({ kind: 'checking' })
    runCheck(defaultUpdaterApi)
      .then(settleCheck)
      .catch((e) =>
        setUpdateState(
          e instanceof UpdateFlowError
            ? e.state
            : { kind: 'error', errorKind: 'unknown', message: String(e) },
        ),
      )
  }

  const installUpdate = () => {
    if (updateResource == null) return
    setUpdateState({ kind: 'downloading', received: 0, total: null })
    runInstall(defaultUpdaterApi, updateResource, (received, total) =>
      setUpdateState({ kind: 'downloading', received, total }),
    )
      .then(() => {
        // One click = download + apply + relaunch; the new process takes over
        // after the updater swaps the bundle.
        defaultUpdaterApi.relaunch()
      })
      .catch((e) =>
        setUpdateState(
          e instanceof UpdateFlowError
            ? e.state
            : { kind: 'error', errorKind: 'unknown', message: String(e) },
        ),
      )
  }

  // The quiet startup check (#66): one fire-and-forget probe after boot. Only
  // a FOUND update becomes visible (the banner); every error — offline, no
  // latest.json, unconfigured pubkey — stays invisible here, while the same
  // probe run from Settings reports its state honestly.
  useEffect(() => {
    runCheck(defaultUpdaterApi)
      .then((result) => {
        if (result.update != null) {
          setUpdateResource(result.update)
          setUpdateState(result.state)
        }
      })
      .catch(() => {
        // Quiet at boot, by design.
      })
  }, [])

  // --- cmux import wizard (#59, HITL rework 2026-08-30) -----------------------
  // Settings → Import → "from cmux" opens the CmuxImportWizard dialog:
  // scan (read-only) → choose categories with a LIVE preview → apply. The
  // shell's glue is exactly two things: the read-only source read, and the
  // persist for Apply — which ALSO closes the Settings dialog (PO call:
  // Apply = done, straight back to the app; the imported rows in the
  // sidebar are the confirmation).
  const [wizardOpen, setWizardOpen] = useState(false)
  const readWizardSources = () =>
    invoke<{ config: string | null; session: string | null }>('read_cmux_import_sources')
  const applyWizardState = (planned: WorkspaceState) => {
    persist(planned)
    setSettingsOpen(false)
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
        // A denied destroy (#57: the allow-destroy capability was missing, so
        // the prevented close silently became "X does nothing") must reach the
        // console — the same rule persist() follows for a rejected save.
        void getCurrentWindow()
          .destroy()
          .catch((e) => console.error('window destroy failed:', e))
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
  // events would otherwise close the menu in the same instant. Clicks INSIDE
  // the menu are ignored here too: React's stopPropagation never reaches the
  // window listener (it only prunes the synthetic tree), so a native click on
  // "Move to group…" — which must SWAP the menu for the picker, not close it
  // — used to tear the whole menu down immediately (#49 round 2).
  useEffect(() => {
    if (menu == null) return
    const close = (e: MouseEvent) => {
      if (Date.now() - menuOpenedAtRef.current < 300) return
      if (
        menuRef.current != null &&
        e.target instanceof Node &&
        menuRef.current.contains(e.target)
      ) {
        return
      }
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
    // The tree rides along since #48: groups + the shared display order are
    // persisted beside the workspace definitions.
    invoke('save_workspaces', {
      workspaces: next.workspaces,
      groups: next.groups,
      order: next.order,
    }).catch((e) => console.error('save_workspaces failed:', e))
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
    await invoke('save_workspaces', {
      workspaces: merged.workspaces,
      groups: merged.groups,
      order: merged.order,
    }).catch((e) => {
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
      case 'toggle-zoom':
        // #40 / story 48: zoom is pure runtime view state — setState, NOT
        // persist (no config write, nothing to save).
        if (activeId != null) setState(toggleZoom(stateRef.current, activeId))
        break
    }
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const cmd = matchShortcut({
        key: e.key,
        code: e.code,
        ctrlKey: e.ctrlKey,
        altKey: e.altKey,
        shiftKey: e.shiftKey,
        metaKey: e.metaKey,
        // activeTagOf: xterm's helper textarea IS the terminal — shortcuts
        // must keep working with focus inside a panel (#40 HITL).
        activeTag: activeTagOf(document.activeElement),
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
    setCreatingKind('workspace')
    setEditingId(null)
    setEditingGroupId(null)
  }

  /// The "New group" action (#48): the same inline-field pattern, appending
  /// an empty group at the end of the top level.
  const startCreateGroup = () => {
    setCreatingKind('group')
    setEditingId(null)
    setEditingGroupId(null)
  }

  const handleCreate = () => {
    const name = draftName.trim()
    if (name === '') return
    persist(
      creatingKind === 'group'
        ? createGroup(state, name)
        : createWorkspace(state, name),
    )
    setDraftName('')
    setCreatingKind(null)
  }

  /// Cancel creation (#37, generalized in #48): hide the name form and drop
  /// the draft. The visible × button and the Escape key share this one path.
  const cancelCreate = () => {
    setCreatingKind(null)
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
    groupId?: string,
  ) => {
    e.preventDefault()
    e.stopPropagation()
    // #43: the menu and the ports tooltip must never share pixels — the
    // tooltip painted OVER the menu's first items. Opening any context menu
    // dismisses a live tip (and any in-flight ports pull); it only comes
    // back on the next fresh hover of a row.
    holdPortsTip()
    portsSeq.current += 1
    setPortsTip(null)
    menuOpenedAtRef.current = Date.now()
    setMenu({ x: e.clientX, y: e.clientY, header, workspaceId, tabId, groupId })
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

  /// Open the color swatch picker (#69): swaps the menu's contents IN PLACE
  /// for the eight palette swatches + clear (the "Move to group…" pattern —
  /// stopPropagation on the entry keeps the close-on-click from firing).
  /// The target is whichever node the menu was opened FROM (#70): a tab, a
  /// workspace, or a group.
  const openColorPicker = () => {
    const target = menu?.tabId ?? menu?.workspaceId ?? menu?.groupId
    if (target == null) return
    setMenu((m) => (m == null ? m : { ...m, colorPickerFor: target }))
  }

  /// The CURRENT color of the picker's target node, whichever kind it is —
  /// the "picked — again to clear" read for the swatch rows.
  const colorOfMenuTarget = (targetId: string): string | null => {
    if (menu?.tabId != null) {
      for (const w of stateRef.current.workspaces) {
        const tab = w.tabs?.find((t) => t.id === targetId)
        if (tab != null) return tab.color ?? null
      }
      return null
    }
    if (stateRef.current.groups.some((g) => g.id === targetId)) {
      return stateRef.current.groups.find((g) => g.id === targetId)?.color ?? null
    }
    return stateRef.current.workspaces.find((w) => w.id === targetId)?.color ?? null
  }

  /// Pick a swatch — or clear (#69, #70). Picking the ALREADY chosen swatch
  /// again unsets (toggle), as does "Clear color"; the setter drops the key
  /// on clear so the payload stays byte-identical to a never-colored node.
  /// The picker target decides the model path: tab, group, or workspace.
  const pickWorkspaceColor = (color: string | null) => {
    const targetId = menu?.colorPickerFor
    const tabId = menu?.tabId
    setMenu(null)
    if (targetId == null) return
    const toggle = (current: string | null) => (current === color ? null : color)
    if (tabId != null) {
      // Tab path (#70): the menu was opened from a TERMINAL TAB; its owning
      // workspace is the menu's workspaceId.
      const wsId = menu?.workspaceId
      if (wsId == null) return
      const current = colorOfMenuTarget(targetId)
      persist(setTabColor(stateRef.current, wsId, tabId, toggle(current)))
      return
    }
    if (stateRef.current.groups.some((g) => g.id === targetId)) {
      const current = colorOfMenuTarget(targetId)
      persist(setGroupColor(stateRef.current, targetId, toggle(current)))
      return
    }
    persist(setWorkspaceColor(stateRef.current, targetId, toggle(colorOfMenuTarget(targetId))))
  }

  // --- Group actions (#48; completed in #51/#52) ------------------------------
  // Inline rename (pencil icon or the group menu), Pin/Unpin (#52), Unpack
  // (#51: dissolve — the children return to top level), and the destructive
  // Delete: a BARE group deletes outright, anything else goes through the
  // SHARED confirmation dialog (#51) with the affected-workspace count and a
  // live-process warning.

  const startGroupRename = (g: Group) => {
    setEditingGroupId(g.id)
    setEditGroupName(g.name)
  }

  const commitGroupRename = (id: string) => {
    const name = editGroupName.trim()
    setEditingGroupId(null)
    if (name !== '') persist(renameGroup(stateRef.current, id, name))
  }

  const renameGroupFromMenu = () => {
    const gid = menu?.groupId
    setMenu(null)
    if (gid == null) return
    const g = stateRef.current.groups.find((x) => x.id === gid)
    if (g != null) startGroupRename(g)
  }

  const deleteGroupFromMenu = () => {
    const gid = menu?.groupId
    setMenu(null)
    if (gid == null) return
    requestDeleteGroup(gid)
  }

  /// Unpack from the group menu (#51): dissolve the group — every workspace
  /// and subgroup it holds returns to top level, nothing closes.
  const unpackGroupFromMenu = () => {
    const gid = menu?.groupId
    setMenu(null)
    if (gid == null) return
    persist(unpackGroup(stateRef.current, gid))
  }

  /// Pin/unpin a group from its context menu (#52): a pinned group leads at
  /// its own level (setGroupPinned keeps the ordering).
  const togglePinGroupFromMenu = () => {
    const gid = menu?.groupId
    setMenu(null)
    if (gid == null) return
    const g = stateRef.current.groups.find((x) => x.id === gid)
    persist(setGroupPinned(stateRef.current, gid, g?.pinned !== true))
  }

  // --- Batch selection actions (#53) -----------------------------------------
  // The selection menu applies each action to EVERY selected node through the
  // workspaces.ts batch ops. The destructive ones (Close all with live
  // processes, Delete all) resolve to ONE shared confirmation naming the
  // total affected workspaces and the live-process count — the same dialog
  // the single-node paths use.

  /// Pin all / Unpin all (#53): every selected node gets the same flag. The
  /// menu shows "Pin all" while ANY member is unpinned, else "Unpin all".
  const batchPinFromMenu = (pinned: boolean) => {
    const ids = menuBatchIds
    setMenu(null)
    if (ids == null) return
    persist(setNodesPinned(stateRef.current, ids, pinned))
    setSelectedIds([])
  }

  /// Close all (#53): closes every selected WORKSPACE (groups are skipped —
  /// closing is a workspace concept). One busy check across all their local
  /// panels; any live process opens the shared confirmation, none closes
  /// immediately — the same ask-or-close contract the single close uses.
  const batchCloseFromMenu = () => {
    const ids = menuBatchIds
    setMenu(null)
    if (ids == null) return
    const wsIds = ids.filter((id) =>
      stateRef.current.workspaces.some((w) => w.id === id),
    )
    if (wsIds.length === 0) return
    const locals = wsIds.flatMap((id) =>
      panelIdsOf(stateRef.current, id)
        .map((pid) => ptyIdsRef.current.get(pid))
        .filter((e): e is PtyEntry => e != null && e.kind === 'local'),
    )
    const finish = () => {
      setState(closeWorkspaces(stateRef.current, wsIds))
      setSelectedIds([])
      void snapshotAndPersist()
    }
    if (locals.length === 0) {
      finish()
      return
    }
    void Promise.all(
      locals.map((e) =>
        invoke<boolean>('pty_is_busy', { id: e.id }).catch(() => false),
      ),
    ).then((results) => {
      const busyCount = results.filter(Boolean).length
      if (busyCount === 0) {
        finish()
        return
      }
      setPendingClose({ kind: 'batch-close', workspaceIds: wsIds, busyCount })
    })
  }

  /// Delete all (#53): resolves the selection to the affected workspace set
  /// (selected groups contribute their whole subtrees) and opens ONE shared
  /// confirmation with the total count and the live-process warning. Confirm
  /// applies deleteNodes to the original selection — overlapping members (a
  /// group AND a workspace inside it) are deduped by the ops themselves.
  const batchDeleteFromMenu = () => {
    const ids = menuBatchIds
    setMenu(null)
    if (ids == null) return
    const affected = new Set<string>()
    for (const id of ids) {
      if (stateRef.current.groups.some((g) => g.id === id)) {
        for (const nodeId of groupSubtreeIds(stateRef.current, id)) {
          if (stateRef.current.workspaces.some((w) => w.id === nodeId)) {
            affected.add(nodeId)
          }
        }
      } else if (stateRef.current.workspaces.some((w) => w.id === id)) {
        affected.add(id)
      }
    }
    const workspaceCount = batchDeleteWorkspaceCount(stateRef.current, ids)
    if (workspaceCount === 0 && affected.size === 0) {
      setSelectedIds([])
      return
    }
    const locals = [...affected].flatMap((id) =>
      panelIdsOf(stateRef.current, id)
        .map((pid) => ptyIdsRef.current.get(pid))
        .filter((e): e is PtyEntry => e != null && e.kind === 'local'),
    )
    if (locals.length === 0) {
      setPendingBatchDelete({ ids, workspaceCount, busyCount: 0 })
      return
    }
    void Promise.all(
      locals.map((e) =>
        invoke<boolean>('pty_is_busy', { id: e.id }).catch(() => false),
      ),
    ).then((results) => {
      setPendingBatchDelete({
        ids,
        workspaceCount,
        busyCount: results.filter(Boolean).length,
      })
    })
  }

  const confirmBatchDelete = () => {
    const pending = pendingBatchDelete
    setPendingBatchDelete(null)
    if (pending == null) return
    persist(deleteNodes(stateRef.current, pending.ids))
    setSelectedIds([])
    void snapshotAndPersist()
  }

  /// Ask-or-delete for a group (#51): a BARE group (nothing inside at any
  /// depth) deletes outright — nothing can be lost. Anything else opens the
  /// shared confirmation, but only after counting how many of the affected
  /// workspaces' LOCAL panels have a running process (same pty_is_busy
  /// batch the workspace close uses) so the dialog can warn.
  const requestDeleteGroup = (gid: string) => {
    const group = stateRef.current.groups.find((g) => g.id === gid)
    if (group == null) return
    if (isGroupEmpty(stateRef.current, gid)) {
      persist(deleteGroup(stateRef.current, gid))
      return
    }
    const affected = stateRef.current.workspaces.filter(
      (w) => w.groupId != null && groupSubtreeIds(stateRef.current, gid).includes(w.groupId),
    )
    const locals = affected.flatMap((w) =>
      panelIdsOf(stateRef.current, w.id)
        .map((pid) => ptyIdsRef.current.get(pid))
        .filter((e): e is PtyEntry => e != null && e.kind === 'local'),
    )
    if (locals.length === 0) {
      setPendingDeleteGroup({
        groupId: gid,
        name: group.name,
        workspaceCount: affected.length,
        busyCount: 0,
      })
      return
    }
    void Promise.all(
      locals.map((e) =>
        invoke<boolean>('pty_is_busy', { id: e.id }).catch(() => false),
      ),
    ).then((results) => {
      setPendingDeleteGroup({
        groupId: gid,
        name: group.name,
        workspaceCount: affected.length,
        busyCount: results.filter(Boolean).length,
      })
    })
  }

  const confirmDeleteGroup = () => {
    const pending = pendingDeleteGroup
    setPendingDeleteGroup(null)
    if (pending == null) return
    persist(deleteGroupSubtree(stateRef.current, pending.groupId))
    void snapshotAndPersist()
  }

  // --- Move to group… (#49) ---------------------------------------------------
  // The workspace menu's "Move to group…" swaps the menu's contents for the
  // picker IN PLACE (same coordinates): one button per top-level group (the
  // current group is left out — moving there is a no-op), plus the inline
  // fresh-name field that creates a group and files the workspace into it.

  // Batch mode for the context menu (#53): opened from a row that is part of
  // a MULTI selection (and the selection still resolves to live nodes), the
  // menu shows the batch actions instead of the single-node ones. A
  // right-click on an unselected row stays a single-node menu.
  const menuBatchIds =
    menu != null &&
    menu.tabId == null &&
    menu.groupPickerFor == null &&
    ((menu.workspaceId != null && selectedIds.includes(menu.workspaceId)) ||
      (menu.groupId != null && selectedIds.includes(menu.groupId))) &&
    selectedIds.filter(
      (id) =>
        state.groups.some((g) => g.id === id) ||
        state.workspaces.some((w) => w.id === id),
    ).length > 1
      ? selectedIds.filter(
          (id) =>
            state.groups.some((g) => g.id === id) ||
            state.workspaces.some((w) => w.id === id),
        )
      : null
  const openGroupPicker = () => {
    const wsId = menu?.workspaceId ?? menu?.groupId
    if (wsId == null) return
    // The batch selection menu's picker (#53) moves the whole selection.
    setMenu((m) =>
      m == null ? m : { ...m, groupPickerFor: wsId, batchMove: menuBatchIds != null },
    )
    setNewGroupName('')
  }

  const pickExistingGroup = (groupId: string) => {
    const wsId = menu?.groupPickerFor
    const batch = menu?.batchMove === true
    setMenu(null)
    if (wsId == null) return
    if (batch) {
      persist(moveNodes(stateRef.current, selectedIdsRef.current, { parentId: groupId }))
      setSelectedIds([])
      return
    }
    persist(moveNode(stateRef.current, wsId, { parentId: groupId }))
  }

  const pickNewGroup = () => {
    const wsId = menu?.groupPickerFor
    const batch = menu?.batchMove === true
    const name = newGroupName.trim()
    setMenu(null)
    setNewGroupName('')
    if (wsId == null || name === '') return
    if (batch) {
      // Create the group on the fly, then file the WHOLE selection into it
      // (#53) — the batch twin of moveToNewGroup.
      const created = createGroup(stateRef.current, name)
      const gid = created.groups[created.groups.length - 1].id
      persist(moveNodes(created, selectedIdsRef.current, { parentId: gid }))
      setSelectedIds([])
      return
    }
    persist(moveToNewGroup(stateRef.current, wsId, name))
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

  // Zoom a panel from its own chrome button (#40): the button zooms the panel
  // it sits on, so focus lands there FIRST and the toggle names the panel
  // explicitly (the shortcut path targets the focused panel instead). One
  // composed update — focus then toggle — over the freshest state.
  const toggleWorkspaceZoom = (id: string, panelId: string) => {
    const focused = focusPanel(stateRef.current, id, panelId)
    setState(toggleZoom(focused, id, panelId))
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
  // Trailing debounce so a burst of openings (boot with several tabs, a split)
  // collapses into ONE snapshot — event-driven (fires on shells REPORTING
  // open), never a poll. This is also what makes branch labels (#41) appear
  // promptly: brand-new panels have no recorded workingDirectory until a
  // snapshot lands; without this they'd wait for the next 20 s periodic tick.
  const openSnapshotTimer = useRef<number | null>(null)
  const notePanelOpened = useCallback(
    (panelId: string, ptyId: number, remote: boolean) => {
      ptyIdsRef.current.set(panelId, { kind: remote ? 'ssh' : 'local', id: ptyId })
      if (openSnapshotTimer.current != null) {
        window.clearTimeout(openSnapshotTimer.current)
      }
      openSnapshotTimer.current = window.setTimeout(() => {
        openSnapshotTimer.current = null
        void snapshotAndPersist()
      }, 150)
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  )

  // A close waiting on the user's confirmation (null = none pending).
  // Rendered by CloseConfirmDialog.
  type PendingClose =
    | { kind: 'panel'; workspaceId: string; panelId: string; label: string }
    | { kind: 'tab'; workspaceId: string; tabId: string; label: string; busyCount: number }
    // Batch close of a multi-selection (#53): one confirmation for every
    // selected workspace's live processes.
    | { kind: 'batch-close'; workspaceIds: string[]; busyCount: number }
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
  const confirmPendingClose = () => {
    const pc = pendingClose
    setPendingClose(null)
    if (pc == null) return
    if (pc.kind === 'panel') doClosePanel(pc.workspaceId, pc.panelId)
    else if (pc.kind === 'tab') {
      persist(closeTab(stateRef.current, pc.workspaceId, pc.tabId))
      void snapshotAndPersist()
    } else if (pc.kind === 'batch-close') {
      setState(closeWorkspaces(stateRef.current, pc.workspaceIds))
      setSelectedIds([])
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

  // A GROUP delete waiting on the shared confirmation (#51): how many
  // workspaces the subtree holds and how many of their panels have live
  // processes — the same dialog the workspace close/delete renders.
  const [pendingDeleteGroup, setPendingDeleteGroup] = useState<{
    groupId: string
    name: string
    workspaceCount: number
    busyCount: number
  } | null>(null)

  // A BATCH delete waiting on the same shared confirmation (#53): the whole
  // selection's ids, the total affected workspace count (subtrees deduped)
  // and the live-process count.
  const [pendingBatchDelete, setPendingBatchDelete] = useState<{
    ids: string[]
    workspaceCount: number
    busyCount: number
  } | null>(null)

  // Dragging a divider (story 18) updates state live (cheap, in-memory) while
  // the final ratio is persisted once, on pointer-up — one config write per
  // drag, not one per pointer-move. stateRef always holds the latest state.
  const stateRef = useRef(state)
  stateRef.current = state

  // Which directory each tab's branch label comes from (focused panel of the
  // active tab, first panel otherwise; null entries yield no label). Same
  // session-restore rule PanelSurfaces applies to the cwd prop.
  const tabBranchDirs = branchDirsByTab(state, settings.sessionRestoreEnabled)

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
  // Command-submitted cwd refresh (#44): the branch label must follow the
  // shell's LIVE directory — after `cd ..` out of a repository, waiting for
  // the 20s periodic tick left the old branch hanging on the row. Enter is
  // the one signal the frontend gets that a command just ran; a short
  // trailing debounce (one timer app-wide, last Enter wins) lets the shell
  // apply the command, then ONE snapshot reads every panel's cwd. Plain
  // keystrokes (submitted=false) arm nothing.
  const commandSnapshotTimer = useRef<number | null>(null)
  const notePanelUserInput = useCallback(
    (panelId: string, submitted: boolean) => {
      applySignal(panelId, 'input', submitted)
      if (!submitted) return
      if (commandSnapshotTimer.current != null) {
        window.clearTimeout(commandSnapshotTimer.current)
      }
      commandSnapshotTimer.current = window.setTimeout(() => {
        commandSnapshotTimer.current = null
        void snapshotAndPersist()
      }, 400)
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    // is-sidebar-resizing (drag resize): col-resize cursor + no text
    // selection for the whole shell while the edge is being dragged.
    <div
      className={
        collapsed
          ? resizingSidebar
            ? 'shell is-sidebar-collapsed is-sidebar-resizing'
            : 'shell is-sidebar-collapsed'
          : resizingSidebar
            ? 'shell is-sidebar-resizing'
            : 'shell'
      }
    >
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
          zero (is-collapsed) instead of unmounting, so the slide is possible.
          The drag handle (HITL 2026-08-30) sits on its right edge; a dragged
          width is applied inline ONLY while expanded, so it can never win
          against is-collapsed's width: 0. */}
      <aside
        className={collapsed ? 'sidebar is-collapsed' : resizingSidebar ? 'sidebar is-resizing' : 'sidebar'}
        style={!collapsed && sidebarWidth != null ? { width: sidebarWidth } : undefined}
        onContextMenu={(e) => openMenu(e, false)}
        onMouseDown={(e) => {
          if (isMenuPress(e)) openMenu(e, false)
        }}
        onClick={(e) => {
          // A click on the sidebar BACKGROUND (not on a row, button or
          // input) clears the multi-selection (#53) — the mouse-driven twin
          // of Escape.
          if (
            (e.target as HTMLElement).closest('.workspace-row, button, input') ==
            null
          ) {
            setSelectedIds([])
          }
        }}
      >
        {/* Drag handle on the right edge (HITL 2026-08-30): widen/narrow the
            sidebar. Separate gesture from collapse/expand; runtime-only. */}
        {!collapsed && (
          <div
            className="sidebar-resizer"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize sidebar"
            title="Drag to resize the sidebar"
            data-testid="sidebar-resizer"
            onPointerDown={onSidebarResizeStart}
            onPointerMove={onSidebarResizeMove}
            onPointerUp={onSidebarResizeEnd}
            onPointerCancel={onSidebarResizeEnd}
          />
        )}
        {/* Fixed-width inner column (#39): the sidebar animates its width;
            this wrapper holds 240px so contents never reflow mid-slide — it
            follows a dragged width so rows use the full sidebar. */}
        <div
          className="sidebar-inner"
          style={!collapsed && sidebarWidth != null ? { width: sidebarWidth } : undefined}
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
                aria-label="Settings"
                title="Settings"
                onClick={() => setSettingsOpen(true)}
              >
                <SettingsIcon />
              </button>
              {/* ONE create button (round 2, Adam): the "+" unfolds a small
                  dropdown offering workspace or group — same inline-field
                  pattern either way. */}
              <div className="header-create" ref={createMenuRef}>
                <button
                  className="icon-btn"
                  aria-label="Add workspace or group"
                  title="Add workspace or group"
                  onClick={() => setCreateMenuOpen((o) => !o)}
                >
                  <PlusIcon />
                </button>
                {createMenuOpen && (
                  <div className="create-dropdown" role="menu">
                    <button
                      className="menu-item"
                      role="menuitem"
                      onClick={() => {
                        setCreateMenuOpen(false)
                        startCreate()
                      }}
                    >
                      <PlusIcon />
                      New workspace
                    </button>
                    <button
                      className="menu-item"
                      role="menuitem"
                      onClick={() => {
                        setCreateMenuOpen(false)
                        startCreateGroup()
                      }}
                    >
                      <FolderPlusIcon />
                      New group
                    </button>
                  </div>
                )}
              </div>
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

        {creatingKind != null && (
          <div className="create-form">
            <input
              className="text-input"
              aria-label={creatingKind === 'group' ? 'New group name' : 'New workspace name'}
              autoFocus
              value={draftName}
              placeholder={creatingKind === 'group' ? 'group name' : 'workspace name'}
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
              aria-label="Cancel creating"
              title="Cancel"
              onClick={cancelCreate}
            >
              <CloseIcon />
            </button>
          </div>
        )}

        {/* position: relative (CSS) — the live drag line anchors here. */}
        <ul className="workspace-list" ref={listRef}>
          {/* The sidebar renders the FLATTENED tree (#48): groups and their
              workspaces interleaved in the shared order, depth driving the
              indentation. */}
          {flattenSidebar(state).map((entry) =>
            entry.kind === 'group' ? (
              <li
                key={entry.group.id}
                data-testid={`group-row-${entry.group.id}`}
                data-node-id={entry.group.id}
                data-testid-collapse={entry.group.collapsed === true ? 'collapsed' : undefined}
                className={`workspace-row group-row ${
                  sideDrop?.intoGroupId === entry.group.id ? 'is-drop-target' : ''
                } ${drag?.kind === 'sidebar' && drag.ids.includes(entry.group.id) ? 'is-dragged' : ''} ${
                  selectedIds.includes(entry.group.id) ? 'is-selected' : ''
                }`}
                style={
                  entry.depth > 0 ? { paddingLeft: 8 + entry.depth * 16 } : undefined
                }
                onPointerDown={(e) => beginSidebarDrag(e, entry.group.id)}
                onClick={(e) => {
                  // A drag's trailing click must not toggle the group.
                  if (suppressClickRef.current) {
                    suppressClickRef.current = false
                    return
                  }
                  // The multi-select modifier (#53) toggles the row in the
                  // selection instead of toggling collapse. The ACTIVE
                  // workspace is default-selected (HITL): an empty selection
                  // seeds with it, so "I'm in ws-1 and click a group" holds
                  // both.
                  if (isMultiSelectModifier(e)) {
                    setSelectedIds((prev) =>
                      prev.length === 0 && state.activeId != null
                        ? [state.activeId, entry.group.id]
                        : toggleInSelection(prev, entry.group.id),
                    )
                    return
                  }
                  setSelectedIds([])
                  // Click toggles collapse IN PLACE (#50) — the flag lives in
                  // the tree (persisted), never in transient UI state.
                  persist(toggleCollapse(stateRef.current, entry.group.id))
                }}
                onContextMenu={(e) =>
                  openMenu(e, false, undefined, undefined, entry.group.id)
                }
                onMouseDown={(e) => {
                  if (isMenuPress(e)) {
                    openMenu(e, false, undefined, undefined, entry.group.id)
                  }
                }}
              >
                {editingGroupId === entry.group.id ? (
                  <input
                    className="text-input"
                    aria-label="Rename group"
                    autoFocus
                    value={editGroupName}
                    onChange={(e) => setEditGroupName(e.target.value)}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') commitGroupRename(entry.group.id)
                      if (e.key === 'Escape') setEditingGroupId(null)
                    }}
                  />
                ) : (
                  <>
                    {/* #52 (Adam's fix): the pin glyph comes BEFORE the
                        folder — a pinned group reads as "pinned, then what
                        kind of node" left to right. */}
                    {entry.group.pinned === true && <PinIcon className="row-pin" />}
                    {/* The folder state doubles as the collapse indicator
                        (#50, Adam's fix): FILLED when the group is collapsed,
                        outline when expanded — same shape, one glance. */}
                    {/* Group color (#70 HITL round): the FOLDER icon is the
                        whole color marker — tinted with the chosen color,
                        no square, no edge. */}
                    {entry.group.collapsed === true ? (
                      <FolderFilledIcon
                        className="row-folder"
                        style={
                          entry.group.color != null
                            ? { color: entry.group.color }
                            : undefined
                        }
                      />
                    ) : (
                      <FolderIcon
                        className="row-folder"
                        style={
                          entry.group.color != null
                            ? { color: entry.group.color }
                            : undefined
                        }
                      />
                    )}
                    <span className="workspace-name">{entry.group.name}</span>
                    {/* Collapsed-group badge (#50): `● N` — how many
                        workspaces in the group's subtree have at least one
                        ACTIVE agent (working, or finished and WAITING for
                        you), aggregated from the same per-panel statuses the
                        chips render (OSC untouched). Hidden entirely when
                        nothing is active. */}
                    {entry.group.collapsed === true &&
                      activeAgentCount(stateRef.current, entry.group.id, statuses) >
                        0 && (
                        <span
                          className="group-badge"
                          data-testid={`group-badge-${entry.group.id}`}
                        >
                          ●{' '}
                          {activeAgentCount(stateRef.current, entry.group.id, statuses)}
                        </span>
                      )}
                    <div className="row-actions">
                      {/* #71 HITL round: the group's inline rename pencil is
                          GONE too — the group menu's "Rename group" is the
                          single rename entry point (same inline edit). */}
                      {/* Delete group (#51): the destructive subtree delete
                          lives behind the shared confirmation — a BARE group
                          (nothing inside) deletes outright, anything else
                          asks with the affected-workspace count and a
                          live-process warning. */}
                      <button
                        className="icon-btn"
                        aria-label={`Delete group ${entry.group.name}`}
                        title="Delete group"
                        onClick={(e) => {
                          e.stopPropagation()
                          requestDeleteGroup(entry.group.id)
                        }}
                      >
                        <CloseIcon />
                      </button>
                    </div>
                  </>
                )}
              </li>
            ) : (
              <li
                key={entry.workspace.id}
                data-testid={`workspace-row-${entry.workspace.id}`}
                data-node-id={entry.workspace.id}
                className={`workspace-row ${
                  entry.workspace.id === state.activeId ? 'is-active' : ''
                } ${drag?.kind === 'sidebar' && drag.ids.includes(entry.workspace.id) ? 'is-dragged' : ''} ${
                  selectedIds.includes(entry.workspace.id) ? 'is-selected' : ''
                }`}
                style={{
                  ...(entry.depth > 0
                    ? { paddingLeft: 8 + entry.depth * 16 }
                    : {}),
                  // Active edge in the workspace's chosen color (#69): ONLY
                  // while this row is the active one — an inactive colored
                  // row keeps the stylesheet's invisible (transparent) edge,
                  // and an uncolored row keeps the default accent.
                  ...(entry.workspace.color != null &&
                  entry.workspace.id === state.activeId
                    ? { borderLeftColor: entry.workspace.color }
                    : {}),
                }}
                onPointerDown={(e) => beginSidebarDrag(e, entry.workspace.id)}
                onClick={(e) => {
                  // A drag's trailing click must not activate the row.
                  if (suppressClickRef.current) {
                    suppressClickRef.current = false
                    return
                  }
                  // The multi-select modifier (#53) toggles the row in the
                  // selection instead of activating the workspace. The
                  // ACTIVE workspace is default-selected (HITL): an empty
                  // selection seeds with it, so "I'm in ws-1 and click ws-2
                  // and ws-3" selects all three. Clicking the active row
                  // itself just selects it.
                  if (isMultiSelectModifier(e)) {
                    setSelectedIds((prev) => {
                      if (prev.length === 0) {
                        const active = state.activeId
                        if (active == null || active === entry.workspace.id) {
                          return [entry.workspace.id]
                        }
                        return [active, entry.workspace.id]
                      }
                      return toggleInSelection(prev, entry.workspace.id)
                    })
                    return
                  }
                  setSelectedIds([])
                  setState(openWorkspace(state, entry.workspace.id))
                }}
                onContextMenu={(e) => openMenu(e, false, entry.workspace.id)}
                // Same hover-pulled ports tooltip as tab rows (#42, round 2):
                // the workspace aggregates EVERY tab it holds into one union,
                // anchored to the sidebar's right wall (round 3).
                onMouseEnter={(e) =>
                  openPortsTip(
                    e,
                    `ws:${entry.workspace.id}`,
                    entry.workspace.tabs ?? [],
                    'right',
                  )
                }
                onMouseLeave={closePortsTip}
                onMouseDown={(e) => {
                  if (isMenuPress(e)) openMenu(e, false, entry.workspace.id)
                }}
              >
                {editingId === entry.workspace.id ? (
                  <input
                    className="text-input"
                    aria-label="Rename workspace"
                    autoFocus
                    value={editName}
                    onChange={(e) => setEditName(e.target.value)}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') commitRename(entry.workspace.id)
                      if (e.key === 'Escape') setEditingId(null)
                    }}
                  />
                ) : (
                  <>
                    {/* Two-line row (#47): the name gets its OWN line and the
                        agent-status chips a second line BELOW it — chips wrap
                        among themselves and can never obscure or squeeze the
                        name at any sidebar width. */}
                    <div className="row-main">
                      <div className="row-name-line">
                        {/* #37: pinned workspaces carry a pin glyph; the
                            pinned group always leads its siblings. On a
                            colored one (#69 HITL round) the pin IS the color
                            marker — tinted, NO square; the square only shows
                            on an UNPINNED colored workspace. */}
                        {entry.workspace.pinned === true && (
                          <PinIcon
                            className="row-pin"
                            style={
                              entry.workspace.color != null
                                ? { color: entry.workspace.color }
                                : undefined
                            }
                          />
                        )}
                        {entry.workspace.color != null &&
                          entry.workspace.pinned !== true && (
                            <span
                              className="row-color-dot"
                              style={{ background: entry.workspace.color }}
                              aria-hidden="true"
                            />
                          )}
                        <span className="workspace-name">{entry.workspace.name}</span>
                      </div>
                      {settings.agentStatusEnabled &&
                        state.openIds.includes(entry.workspace.id) &&
                        (() => {
                          // One chip per panel, the ACTIVE panel's chip first
                          // with the bigger dot (HITL round 5: the big dot
                          // marks the terminal you're in — switching panel
                          // focus moves it), the remaining panels' statuses
                          // as minis below. Since the #37 rework this covers
                          // EVERY tab's panes — all of a workspace's statuses
                          // live here, on its row (Adam's call: tabs stay
                          // clean, no chips on the tab bar).
                          const pids = panelIdsOf(state, entry.workspace.id)
                          if (pids.length === 0) return null
                          const activePid =
                            activePanelOf(state, entry.workspace.id) ?? pids[0]
                          // Fixed chip order (panel tree order) — chips never
                          // move when focus changes; ONLY the active panel's
                          // dot grows (mini -> full), exactly where that
                          // panel's chip sits (HITL round 7).
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
                    </div>
                    <div className="row-actions">
                      {/* #98 / v1.5.0: the inline rename pencil is GONE from
                          workspace rows — the context menu's "Rename
                          workspace" is the single rename entry point (it
                          drives the same inline edit). Group rows keep their
                          pencil. HITL round: the row's × is the DELETE —
                          nothing is left behind, behind the shared
                          confirmation. */}
                      <button
                        className="icon-btn"
                        aria-label={`Delete ${entry.workspace.name}`}
                        title="Delete workspace"
                        onClick={(e) => {
                          e.stopPropagation()
                          setPendingDelete({
                            id: entry.workspace.id,
                            name: entry.workspace.name,
                          })
                        }}
                      >
                        <CloseIcon />
                      </button>
                    </div>
                  </>
                )}
              </li>
            ),
          )}
          {state.workspaces.length === 0 && (
            <li className="empty-hint">No workspaces yet.</li>
          )}
          {/* Live insertion line (round 3): follows the pointer while a
              sidebar row drags; a group's middle zone highlights the row
              instead (the drop FILES into it), so no line then. */}
          {drag?.kind === 'sidebar' && sideDrop?.lineTop != null && (
            <div
              className="drag-line"
              style={{ top: sideDrop.lineTop - drag.listTop }}
            />
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
          {state.workspaces.length === 0 && creatingKind == null ? (
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
                  data-testid={`tab-bar-${ws.id}`}
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
                        data-tab-id={tab.id}
                        className={`tab ${tabActive ? 'is-active' : ''} ${
                          drag?.kind === 'tab' && drag.id === tab.id ? 'is-dragged' : ''
                        }`}
                        // Active tab edge (#70): tabs carry their default
                        // accent as the TOP strip (.tab.is-active's inset
                        // shadow) — a colored active tab recolors that strip;
                        // an inactive one keeps no strip at all.
                        style={
                          tab.color != null && tabActive
                            ? { boxShadow: `inset 0 2px 0 0 ${tab.color}` }
                            : undefined
                        }
                        // Live pointer drag reorder (round 3; same-workspace
                        // only, #45 semantics): press, move, release — the
                        // line follows the pointer between tabs.
                        onPointerDown={(e) => beginTabDrag(e, ws.id, tab.id)}
                        // Listening ports (#42): the custom hover tooltip
                        // replaced the native title here — showing BOTH
                        // would be two competing tooltips over one row.
                        onMouseEnter={(e) => openPortsTip(e, tab.id, [tab])}
                        onMouseLeave={closePortsTip}
                        onClick={() => {
                          // A drag's trailing click must not switch tabs.
                          if (suppressClickRef.current) {
                            suppressClickRef.current = false
                            return
                          }
                          setState(switchTab(state, ws.id, tab.id))
                        }}
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
                        {/* Pinned tab (#37) — and on a colored one (#70 HITL
                            round) the pin IS the color marker: tinted, with
                            NO square beside it. The square only shows on an
                            UNPINNED colored tab. */}
                        {tab.pinned === true && (
                          <PinIcon
                            className="tab-pin"
                            style={
                              tab.color != null ? { color: tab.color } : undefined
                            }
                          />
                        )}
                        {tab.color != null && tab.pinned !== true && (
                          <span
                            className="tab-color-dot"
                            style={{ background: tab.color }}
                            aria-hidden="true"
                          />
                        )}
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
                          <>
                            <span className="tab-name">{label}</span>
                            {/* Git branches (#41, HITL rework 2026-08-27):
                                ONE per panel of this tab — a split shows as
                                many as its splits — and the FOCUSED panel's
                                branch is bold (.tab-branch.is-focused), never
                                larger. Entries without a repository render
                                nothing: no placeholder, ever. */}
                            {(tabBranchDirs[tab.id] ?? []).map((entry) => {
                              const branch =
                                entry.dir != null ? branchLabels[entry.dir] : undefined
                              if (branch == null || branch === '') return null
                              return (
                                <span
                                  key={entry.panelId}
                                  className={
                                    entry.focused ? 'tab-branch is-focused' : 'tab-branch'
                                  }
                                >
                                  {branch}
                                </span>
                              )
                            })}
                          </>
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
                  {/* Live insertion line for tab drags (round 3). */}
                  {drag?.kind === 'tab' && drag.wsId === ws.id && tabDrop != null && (
                    <div
                      className="drag-line-tab"
                      style={{ left: tabDrop.lineLeft - drag.barLeft }}
                    />
                  )}
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
                        focused={isActive}
                        firstLeafId={leafIds(tab.layout)[0] ?? ''}
                        zoomedPanelId={zoomedPanelOf(state, tab.id)}
                        onToggleZoom={(panelId) => toggleWorkspaceZoom(ws.id, panelId)}
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
          onOpenSettingsFile={openSettingsFile}
          onImportWizard={() => setWizardOpen(true)}
          updates={{
            state: updateState,
            onCheck: checkForUpdates,
            onInstall: installUpdate,
          }}
        />
      )}

      {/* Update banner (issue #66): appears only when a check FOUND an update
          (updateResource != null) — quiet states never raise it. It follows
          the same state machine as the Settings row: the one-click install,
          its progress, and a failure readout with retry. */}
      {updateResource != null && !updateBannerDismissed && (
        <div className="update-banner" role="status" data-testid="update-banner">
          {updateState.kind === 'downloading' ? (
            <span>
              Downloading update —{' '}
              {downloadProgressText(updateState.received, updateState.total)}
            </span>
          ) : updateState.kind === 'error' ? (
            <>
              <span className="update-banner__error">{updateState.message}</span>
              <button type="button" className="btn-primary" onClick={installUpdate}>
                Retry
              </button>
            </>
          ) : (
            <>
              <span>
                umux {updateState.kind === 'available' ? updateState.version : ''} is
                available.
              </span>
              <button type="button" className="btn-primary" onClick={installUpdate}>
                Download &amp; restart
              </button>
            </>
          )}
          <button
            type="button"
            className="icon-btn"
            aria-label="Dismiss update banner"
            title="Dismiss"
            onClick={() => setUpdateBannerDismissed(true)}
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
            >
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </button>
        </div>
      )}

      {/* The "from cmux" import wizard (#59): scan → choose + live preview →
          apply. stateRef.current is captured at open time — the modal blocks
          the app behind it, so the preview cannot race the live tree. Apply
          persists AND closes Settings (applyWizardState), so both dialogs
          leave together. */}
      {wizardOpen && (
        <CmuxImportWizard
          liveState={stateRef.current}
          onReadSources={readWizardSources}
          onApply={applyWizardState}
          onClose={() => setWizardOpen(false)}
        />
      )}

      {pendingClose != null && (
        <CloseConfirmDialog
          title={
            pendingClose.kind === 'panel'
              ? 'Close this panel?'
              : pendingClose.kind === 'tab'
                ? 'Close this tab?'
                : `Close ${pendingClose.workspaceIds.length} workspaces?`
          }
          message={
            pendingClose.kind === 'panel'
              ? `Panel ${pendingClose.label} has a running process. Closing it now will terminate that process.`
              : pendingClose.kind === 'tab'
                ? `${pendingClose.busyCount} panel${pendingClose.busyCount === 1 ? '' : 's'} in tab ${pendingClose.label} ha${pendingClose.busyCount === 1 ? 's' : 've'} a running process. Closing it now will terminate ${pendingClose.busyCount === 1 ? 'it' : 'them'}.`
                : `${pendingClose.busyCount} panel${pendingClose.busyCount === 1 ? '' : 's'} in the selected workspaces ha${pendingClose.busyCount === 1 ? 's' : 've'} a running process. Closing them now will terminate ${pendingClose.busyCount === 1 ? 'it' : 'them'}.`
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

      {/* Shared destructive confirmation (#51): Delete group removes the
          group with EVERYTHING inside it. The message names how many
          workspaces are affected and warns when any has a live process —
          the same contract #53's batch actions will reuse. */}
      {pendingDeleteGroup != null && (
        <CloseConfirmDialog
          title={`Delete group "${pendingDeleteGroup.name}"?`}
          message={`Delete "${pendingDeleteGroup.name}" with everything inside it? ${
            pendingDeleteGroup.workspaceCount
          } workspace${pendingDeleteGroup.workspaceCount === 1 ? '' : 's'} will be removed. This cannot be undone.${
            pendingDeleteGroup.busyCount > 0
              ? ` ${pendingDeleteGroup.busyCount} panel${
                  pendingDeleteGroup.busyCount === 1 ? ' has' : 's have'
                } a running process that will be terminated.`
              : ''
          }`}
          confirmLabel="Delete"
          onConfirm={confirmDeleteGroup}
          onCancel={() => setPendingDeleteGroup(null)}
        />
      )}

      {/* Batch delete confirmation (#53): ONE dialog for the whole
          selection — total affected workspaces (deduped subtrees) and the
          live-process warning, the same contract as the single/group delete. */}
      {pendingBatchDelete != null && (
        <CloseConfirmDialog
          title={`Delete ${pendingBatchDelete.workspaceCount} workspace${pendingBatchDelete.workspaceCount === 1 ? '' : 's'}?`}
          message={`Delete the selection with everything inside it? ${
            pendingBatchDelete.workspaceCount
          } workspace${pendingBatchDelete.workspaceCount === 1 ? '' : 's'} will be removed. This cannot be undone.${
            pendingBatchDelete.busyCount > 0
              ? ` ${pendingBatchDelete.busyCount} panel${
                  pendingBatchDelete.busyCount === 1 ? ' has' : 's have'
                } a running process that will be terminated.`
              : ''
          }`}
          confirmLabel="Delete"
          onConfirm={confirmBatchDelete}
          onCancel={() => setPendingBatchDelete(null)}
        />
      )}

      {menu && (
        <div
          className="context-menu"
          ref={menuRef}
          // Position: the pointer's coordinates, PULLED BACK inside the
          // viewport once the menu's real size is known (menuPos; the raw
          // coordinates render for the single pre-paint pass only).
          style={{
            left: menuPos?.left ?? menu.x,
            top: menuPos?.top ?? menu.y,
          }}
          role="menu"
          // Any click inside the menu (item or padding) closes it — the
          // window-level close listener alone is not enough now that it is
          // time-guarded against the opening gesture.
          onClick={() => setMenu(null)}
        >
          {menu.groupPickerFor != null ? (
            /* "Move to group…" picker (#49): swaps the menu's contents IN
                PLACE — one button per top-level group (the workspace's
                current group is left out; moving there is a no-op), plus the
                fresh-name field that creates a group on the fly and files
                the workspace into it. Clicks inside stay local — only an
                item, Enter, or an outside click closes. */
            <div className="menu-picker" onClick={(e) => e.stopPropagation()}>
              <div className="menu-picker-title">Move to group…</div>
              {state.groups
                .filter(
                  (g) =>
                    g.id !==
                    state.workspaces.find((w) => w.id === menu.groupPickerFor)
                      ?.groupId,
                )
                .map((g) => (
                  <button
                    key={g.id}
                    className="menu-item"
                    role="menuitem"
                    onClick={() => pickExistingGroup(g.id)}
                  >
                    <FolderIcon />
                    {g.name}
                  </button>
                ))}
              {state.groups.length === 0 && (
                <div className="menu-note">No groups yet — name one below.</div>
              )}
              <div className="menu-separator" />
              <input
                className="text-input menu-picker-input"
                aria-label="New group name"
                placeholder="new group name"
                autoFocus
                value={newGroupName}
                onChange={(e) => setNewGroupName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') pickNewGroup()
                  if (e.key === 'Escape') setMenu(null)
                }}
              />
              <button className="menu-item" role="menuitem" onClick={pickNewGroup}>
                <FolderPlusIcon />
                Create group and move
              </button>
            </div>
          ) : menu.colorPickerFor != null ? (
            /* Color swatch picker (#69): the eight fixed palette swatches
                plus the clear entry, swapped IN PLACE like the group picker.
                Every row is an ordinary interactive menu-item — the shared
                class carries the press feedback (story #101 standing rule). */
            <div className="menu-picker" onClick={(e) => e.stopPropagation()}>
              <div className="menu-picker-title">Color</div>
              {(() => {
                // The picker's target (#69 workspace, #70 tab or group) —
                // the same lookup the pick handler branches on.
                let current: string | null = null
                if (menu.tabId != null) {
                  for (const w of state.workspaces) {
                    const tab = w.tabs?.find((t) => t.id === menu.colorPickerFor)
                    if (tab != null) {
                      current = tab.color ?? null
                      break
                    }
                  }
                } else if (state.groups.some((g) => g.id === menu.colorPickerFor)) {
                  current =
                    state.groups.find((g) => g.id === menu.colorPickerFor)?.color ??
                    null
                } else {
                  current =
                    state.workspaces.find((w) => w.id === menu.colorPickerFor)
                      ?.color ?? null
                }
                return (
                  <>
                    {COLOR_PALETTE.map((c) => (
                      <button
                        key={c.hex}
                        className="menu-item"
                        role="menuitem"
                        aria-label={`Set color ${c.name}`}
                        onClick={() => pickWorkspaceColor(c.hex)}
                      >
                        <span
                          className="menu-color-dot"
                          style={{ background: c.hex }}
                        />
                        {current === c.hex ? `${c.name} (picked — again to clear)` : c.name}
                      </button>
                    ))}
                    <div className="menu-separator" />
                    <button
                      className="menu-item"
                      role="menuitem"
                      aria-label="Clear color"
                      onClick={() => pickWorkspaceColor(null)}
                    >
                      <span className="menu-color-dot menu-color-dot-none" />
                      Clear color
                    </button>
                  </>
                )
              })()}
            </div>
          ) : (
            <>
          {menu.tabId == null && (
            <>
              <button className="menu-item" role="menuitem" onClick={startCreate}>
                <PlusIcon />
                New workspace
              </button>
              {/* New group (#48): the sidebar's group action, reachable from
                  every non-tab menu — same inline-field pattern. */}
              <button className="menu-item" role="menuitem" onClick={() => {
                setMenu(null)
                startCreateGroup()
              }}>
                <FolderPlusIcon />
                New group
              </button>
            </>
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
                  <button
                    className="menu-item"
                    role="menuitem"
                    // stopPropagation: this item SWAPS the menu's contents for
                    // the color picker (#70) instead of closing it.
                    onClick={(e) => {
                      e.stopPropagation()
                      openColorPicker()
                    }}
                  >
                    <ColorIcon />
                    Color
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
          {/* Batch selection menu (#53): opened from a row that belongs to a
              multi-selection. One set of actions for EVERY selected node —
              Move to group…, Pin all/Unpin all, Close all, Delete all (the
              destructive ones resolve to the shared confirmation). Rename is
              hidden here by design: it is a single-node action only. */}
          {menuBatchIds != null &&
            (() => {
              const anyUnpinned = menuBatchIds.some(
                (id) => !isNodePinned(state, id),
              )
              return (
                <>
                  <div className="menu-separator" />
                  <button
                    className="menu-item"
                    role="menuitem"
                    // Swaps the menu for the group picker IN PLACE, with the
                    // batch flag set — the pick applies to the whole
                    // selection.
                    onClick={(e) => {
                      e.stopPropagation()
                      openGroupPicker()
                    }}
                  >
                    <FolderIcon />
                    Move to group…
                  </button>
                  <button
                    className="menu-item"
                    role="menuitem"
                    onClick={() => batchPinFromMenu(anyUnpinned)}
                  >
                    <PinIcon />
                    {anyUnpinned ? 'Pin all' : 'Unpin all'}
                  </button>
                  <div className="menu-separator" />
                  <button
                    className="menu-item"
                    role="menuitem"
                    onClick={batchCloseFromMenu}
                  >
                    <CloseIcon />
                    Close all
                  </button>
                  <button
                    className="menu-item danger"
                    role="menuitem"
                    onClick={batchDeleteFromMenu}
                  >
                    <CloseIcon />
                    Delete all
                  </button>
                </>
              )
            })()}
          {menu.tabId == null && menu.workspaceId && menuBatchIds == null && (() => {
            // #47 (plan Phase 1): the splits LEFT the workspace menu — they
            // stay on the tab menu and the shortcuts, where the panel they
            // split is unambiguous. Pin / Rename / Delete remain (Adam's
            // position: Pin directly before Delete), joined by "Move to
            // group…" (#49).
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
                <button
                  className="menu-item"
                  role="menuitem"
                  // stopPropagation: this item SWAPS the menu's contents for
                  // the color picker (#69) instead of closing it — the
                  // container's close-on-click must not fire over it.
                  onClick={(e) => {
                    e.stopPropagation()
                    openColorPicker()
                  }}
                >
                  <ColorIcon />
                  Color
                </button>
                <button
                  className="menu-item"
                  role="menuitem"
                  // stopPropagation: this item SWAPS the menu's contents for
                  // the picker instead of closing it — the container's
                  // close-on-click must not fire over it.
                  onClick={(e) => {
                    e.stopPropagation()
                    openGroupPicker()
                  }}
                >
                  <FolderIcon />
                  Move to group…
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
          {/* Group row menu (#48; complete since #51/#52): Pin/Unpin, Rename,
              Unpack (dissolve — children return to top level) and the
              destructive Delete, which asks through the shared confirmation
              whenever the group holds anything. */}
          {menu.tabId == null && menu.groupId && menuBatchIds == null && (() => {
            const menuGroup = state.groups.find((g) => g.id === menu.groupId)
            if (menuGroup == null) return null
            const pinned = menuGroup.pinned === true
            return (
              <>
                <div className="menu-separator" />
                <button
                  className="menu-item"
                  role="menuitem"
                  onClick={togglePinGroupFromMenu}
                >
                  <PinIcon />
                  {pinned ? 'Unpin group' : 'Pin group'}
                </button>
                <button
                  className="menu-item"
                  role="menuitem"
                  onClick={renameGroupFromMenu}
                >
                  <PencilIcon />
                  Rename group
                </button>
                <button
                  className="menu-item"
                  role="menuitem"
                  // stopPropagation: this item SWAPS the menu's contents for
                  // the color picker (#70) instead of closing it.
                  onClick={(e) => {
                    e.stopPropagation()
                    openColorPicker()
                  }}
                >
                  <ColorIcon />
                  Color
                </button>
                <button
                  className="menu-item"
                  role="menuitem"
                  onClick={unpackGroupFromMenu}
                >
                  <FolderIcon />
                  Unpack group
                </button>
                <div className="menu-separator" />
                <button
                  className="menu-item danger"
                  role="menuitem"
                  onClick={deleteGroupFromMenu}
                >
                  <CloseIcon />
                  Delete group
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
            </>
          )}
        </div>
      )}
      {/* Listening-ports tooltip (#42): appears under the hovered row once
          its ONE pull answers; the explicit text makes "nothing listens"
          unambiguous. Round 2: the tooltip is enterable (entering it
          cancels the grace-close). Since #72 each port button OPENS its
          localhost URL in the browser and still copies it. */}
      {portsTip && portsTip.ports !== null && (
        <div
          className="tab-ports-tip"
          role="tooltip"
          style={{ left: portsTip.left, top: portsTip.top }}
          onMouseEnter={holdPortsTip}
          onMouseLeave={closePortsTip}
        >
          {portsTip.ports.length === 0 ? (
            formatPorts(portsTip.ports)
          ) : (
            portsTip.ports.map((port, i) => (
              <Fragment key={port}>
                {i > 0 && ' · '}
                <button
                  className="tab-ports-tip-port"
                  title={`Open http://localhost:${port}`}
                  onClick={() => openPort(port)}
                >
                  {port}
                </button>
              </Fragment>
            ))
          )}
        </div>
      )}
      {/* The drag ghost (round 4): a pill carrying the dragged row's name
          travels just under the pointer — the carrier is never invisible.
          React seeds the transform once at activation; the window pointermove
          mutates it imperatively from there. */}
      {drag != null && (
        <div
          className="drag-ghost"
          ref={ghostRef}
          style={{ transform: `translate(${drag.x + 12}px, ${drag.y + 10}px)` }}
        >
          {drag.kind === 'sidebar' && drag.isGroup && (
            <FolderIcon className="drag-ghost-icon" />
          )}
          <span>{drag.label}</span>
        </div>
      )}
    </div>
  )
}
