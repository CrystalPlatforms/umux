// NotificationMuteButton — the notification mute toggle (Phase 14 / #15).
//
// Tests behavior through the component's public interface (props + DOM). No
// mocks: the component is presentational — it owns NO state and makes NO Tauri
// calls. The mute state and the invoke('set_notifications_muted') call live in
// WorkspaceShell (UI glue, verified manually by Adam); this component is the
// small, testable surface: given a `muted` flag it shows the right indicator,
// and a click asks the parent to toggle.
//
// Assumptions encoded (stated before the first RED):
//  - Input:  props { muted: boolean, onToggle: () => void }.
//  - Output: a <button> whose pressed state (aria-pressed) mirrors `muted`
//            (AC3 — the mute state is visible in the UI), whose accessible name
//            reflects the current state, and whose click fires onToggle once
//            (AC1 — the user can toggle off and back on).
//  - NOT tested here: the Tauri command wiring and initial-state load
//    (WorkspaceShell glue, manual).

import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import { NotificationMuteButton } from './NotificationMuteButton'

describe('NotificationMuteButton', () => {
  // T1 (AC3 — mute state is visible in the UI): when audible, the button reports
  //   aria-pressed="false" and names itself as the "mute" action.
  it('shows the unmuted state when muted is false', () => {
    const { getByRole } = render(
      <NotificationMuteButton muted={false} onToggle={() => {}} />,
    )
    const button = getByRole('button')
    expect(button).toHaveAttribute('aria-pressed', 'false')
    expect(button.getAttribute('aria-label')).toMatch(/mute/i)
  })

  // T1b (AC3 — mute state is visible in the UI): when silenced, the button
  //   reports aria-pressed="true".
  it('shows the muted state when muted is true', () => {
    const { getByRole } = render(
      <NotificationMuteButton muted={true} onToggle={() => {}} />,
    )
    expect(getByRole('button')).toHaveAttribute('aria-pressed', 'true')
  })

  // T2 (AC1 — clicking toggles):
  //   Input:  a click on the button.
  //   Output: onToggle is called exactly once.
  it('calls onToggle when clicked', () => {
    const onToggle = vi.fn()
    const { getByRole } = render(
      <NotificationMuteButton muted={false} onToggle={onToggle} />,
    )
    fireEvent.click(getByRole('button'))
    expect(onToggle).toHaveBeenCalledTimes(1)
  })
})
