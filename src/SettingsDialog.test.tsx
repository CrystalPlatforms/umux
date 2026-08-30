// SettingsDialog — the feature-toggle screen (v0.2 Phase 3 / #27).
//
// Tests behavior through the component's public interface (props + DOM). No
// mocks: the component is presentational — it owns NO state and makes NO
// Tauri calls. The settings object, the invoke('save_settings') flow, and the
// immediate-effect wiring live in WorkspaceShell (UI glue, verified manually
// by Adam).
//
// Assumptions encoded (stated before the first RED):
//  - Input:  props { settings: Settings, onChange: (patch) => void,
//            onClose: () => void }.
//  - Output: one role="switch" per feature (aria-checked mirrors the prop);
//            a click fires onChange with the NEXT value for that toggle only;
//            Escape / overlay click / header X fire onClose.
//  - NOT tested here: persistence and live effect (WorkspaceShell glue).

import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import { SettingsDialog } from './SettingsDialog'
import { defaultSettings } from './settings'

describe('SettingsDialog', () => {
  // T1 (AC1 — the screen shows the toggles): one switch per USER-CONTROLLABLE
  //   feature renders, reflecting its settings value through aria-checked.
  //   Analytics is deliberately absent (always-on, no switch — HITL decision).
  it('renders one switch per user-controllable feature, and no analytics switch', () => {
    const { getByTestId, queryByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )

    const notifications = getByTestId('toggle-notifications')
    const agentStatus = getByTestId('toggle-agent-status')
    const sessionRestore = getByTestId('toggle-session-restore')

    expect(notifications).toHaveAttribute('aria-checked', 'true')
    expect(agentStatus).toHaveAttribute('aria-checked', 'true')
    expect(sessionRestore).toHaveAttribute('aria-checked', 'true')
    expect(getByTestId('toggle-ports-tooltip')).toHaveAttribute('aria-checked', 'true')
    expect(queryByTestId('toggle-analytics')).toBeNull()
  })

  // T2 (AC2/AC3 — flipping a switch reports the next value):
  //   Input:  settings with notifications ON; a click on its switch.
  //   Output: onChange fires once with { notificationsEnabled: false } —
  //           the parent applies the patch; the component itself stays dumb.
  it('reports a toggle flip as a patch with the next value', () => {
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={onChange} onClose={() => {}} />,
    )

    fireEvent.click(getByTestId('toggle-notifications'))

    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith({ notificationsEnabled: false })
  })

  // T2b (the same contract for the agent-status toggle, in the other
  //   direction — off -> on):
  it('reports an agent-status flip as a patch with the next value', () => {
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={{ ...defaultSettings, agentStatusEnabled: false }}
        onChange={onChange}
        onClose={() => {}}
      />,
    )

    fireEvent.click(getByTestId('toggle-agent-status'))

    expect(onChange).toHaveBeenCalledWith({ agentStatusEnabled: true })
  })

  // T2c (#43 — the ports-tooltip toggle follows the same contract, both
  //   directions: on -> off reports the patch, off reflects aria-checked).
  it('reports a ports-tooltip flip as a patch with the next value', () => {
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={{ ...defaultSettings, portsTooltipEnabled: true }}
        onChange={onChange}
        onClose={() => {}}
      />,
    )

    fireEvent.click(getByTestId('toggle-ports-tooltip'))

    expect(onChange).toHaveBeenCalledWith({ portsTooltipEnabled: false })
  })

  it('mirrors portsTooltipEnabled=false in the switch state', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={{ ...defaultSettings, portsTooltipEnabled: false }}
        onChange={() => {}}
        onClose={() => {}}
      />,
    )

    expect(getByTestId('toggle-ports-tooltip')).toHaveAttribute('aria-checked', 'false')
  })

  // T3 (analytics is invisible to the user — always on, no switch; the HITL
  //   product decision): the dialog must not mention analytics at all.
  it('does not surface any analytics wording', () => {
    const { queryByText } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )

    expect(queryByText(/analytics/i)).toBeNull()
  })

  // T4 (dismissal — the same "back out" reflex as the rename/create inputs):
  //   Escape, a click on the backdrop, and the header X all fire onClose.
  it('closes on Escape, backdrop click, and the header X', () => {
    const onClose = vi.fn()

    const first = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={onClose} />,
    )
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
    first.unmount()

    const second = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={onClose} />,
    )
    fireEvent.click(second.getByTestId('settings-dialog'))
    expect(onClose).toHaveBeenCalledTimes(2)

    fireEvent.click(second.getByRole('button', { name: /close settings/i }))
    expect(onClose).toHaveBeenCalledTimes(3)
  })

  // T5 (the footnote's settings.json is a LINK): clicking the settings.json
  //   button reports upward (the parent runs the Tauri invoke — this
  //   component stays invoke-free), and the click must NOT close the dialog.
  it('clicking the settings.json link reports it upward without closing', () => {
    const onOpen = vi.fn()
    const onClose = vi.fn()
    const { getByRole } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={onClose}
        onOpenSettingsFile={onOpen}
      />,
    )

    fireEvent.click(getByRole('button', { name: 'settings.json' }))

    expect(onOpen).toHaveBeenCalledTimes(1)
    expect(onClose).not.toHaveBeenCalled()
  })
})


// --- Import dropdown → wizard (#59 rework) -----------------------------------
//
// The "from cmux" item now OPENS the import wizard (rendered by the shell);
// this dialog only reports upward. The item is absent on Windows (v1.2.0
// decision #4). The wizard's own behavior lives in CmuxImportWizard.test.tsx.
describe('SettingsDialog import dropdown (#59 rework)', () => {
  it('the "from cmux" item reports upward instead of importing inline', () => {
    const onImportWizard = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onImportWizard={onImportWizard}
      />,
    )

    fireEvent.click(getByTestId('import-toggle'))
    fireEvent.click(getByTestId('import-cmux'))

    expect(onImportWizard).toHaveBeenCalledTimes(1)
  })

  it('hides the whole import row on Windows', () => {
    Object.defineProperty(window.navigator, 'platform', {
      value: 'Win32',
      configurable: true,
    })
    try {
      const { queryByTestId } = render(
        <SettingsDialog
          settings={defaultSettings}
          onChange={() => {}}
          onClose={() => {}}
          onImportWizard={() => {}}
        />,
      )
      expect(queryByTestId('import-toggle')).toBeNull()
    } finally {
      delete (window.navigator as { platform?: string }).platform
    }
  })
})
