// AgentStatusIndicator — the workspace-row status chip (v0.2 Phase 2 / #26).
//
// Renders a colored dot followed by the human label for ONE aggregate status
// (see aggregateStatus): "Idle" / "Running" / "Needs Input". Pure
// presentational — the machines (agentStatus.ts) decide, WorkspaceShell wires
// the events, this only draws. Styling (dot color, pulse) keys off the
// agent-status--<state> modifier class; see .agent-status in app.css.

import type { AgentStatus } from './agentStatus'

const STATUS_TEXT: Record<AgentStatus, string> = {
  idle: 'Idle',
  working: 'Running',
  'needs-attention': 'Needs Input',
}

export function AgentStatusIndicator({
  status,
  mini = false,
}: {
  status: AgentStatus
  /** Smaller variant for the non-active panels' statuses in the sidebar
   * stack — the active panel's chip renders full-size, in place (the chips
   * themselves never move; HITL round 7). */
  mini?: boolean
}) {
  return (
    <span
      className={`agent-status agent-status--${status}${mini ? ' agent-status--mini' : ''}`}
      role="status"
    >
      <span className="agent-dot" aria-hidden="true" />
      {STATUS_TEXT[status]}
    </span>
  )
}
