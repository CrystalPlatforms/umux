// AgentStatusIndicator — the workspace-row status chip (v0.2 Phase 2 / #26).
//
// Presentational: given one aggregate status it renders a colored dot followed
// by the human label Adam specified: "Idle" / "Running" / "Needs Input". No
// state, no Tauri calls — aggregation lives in agentStatus.ts (unit-tested
// there), the machines and event wiring live in WorkspaceShell (glue, verified
// manually by Adam).
//
// Assumptions encoded (stated before the first RED):
//  - Input:  props { status: AgentStatus }.
//  - Output: one element (role="status") containing a dot span followed by the
//            label text, for ALL THREE states (idle is visible too — the
//            workspace row always shows where things stand). The state also
//            lands in a modifier class for styling.
//  - NOT tested here: colors/animation (CSS, HITL), aggregation (agentStatus).

import { describe, it, expect } from 'vitest'
import { render } from '@testing-library/react'
import { AgentStatusIndicator } from './AgentStatusIndicator'

describe('AgentStatusIndicator', () => {
  // T1 (idle is a visible label now, not the absence of an element):
  it('renders the Idle label with a dot', () => {
    const { getByRole } = render(<AgentStatusIndicator status="idle" />)
    expect(getByRole('status')).toHaveTextContent('Idle')
    expect(getByRole('status').firstElementChild).toHaveClass('agent-dot')
  })

  // T2 (working streams show "Running"):
  it('renders the Running label with the working modifier class', () => {
    const { getByRole } = render(<AgentStatusIndicator status="working" />)
    expect(getByRole('status')).toHaveTextContent('Running')
    expect(getByRole('status')).toHaveClass('agent-status--working')
  })

  // T3 (a finished task shows "Needs Input"):
  it('renders the Needs Input label with the needs-attention modifier class', () => {
    const { getByRole } = render(<AgentStatusIndicator status="needs-attention" />)
    expect(getByRole('status')).toHaveTextContent('Needs Input')
    expect(getByRole('status')).toHaveClass('agent-status--needs-attention')
  })
})
