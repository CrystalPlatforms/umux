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
import { render, fireEvent, screen } from '@testing-library/react'
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

  // #80 / #81 (v1.6.0): the two sidebar-display switches. Branch labels are
  // ON by default (switch ON = visible); folder lines default OFF.
  it('renders the git-branch switch ON and folders switch OFF (#80, #81)', () => {
    const { getByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )

    expect(getByTestId('toggle-show-tab-branch')).toHaveAttribute('aria-checked', 'true')
    expect(getByTestId('toggle-show-tab-folders')).toHaveAttribute('aria-checked', 'false')
    expect(
      getByTestId('toggle-show-tab-branch').getAttribute('aria-label'),
    ).toMatch(/git branch on tab rows/i)
    expect(
      getByTestId('toggle-show-tab-folders').getAttribute('aria-label'),
    ).toMatch(/folder/i)
  })

  it('reports a flip of each new switch as a patch with the next value (#80, #81)', () => {
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={onChange} onClose={() => {}} />,
    )

    fireEvent.click(getByTestId('toggle-show-tab-branch'))
    fireEvent.click(getByTestId('toggle-show-tab-folders'))

    expect(onChange).toHaveBeenNthCalledWith(1, { showTabBranch: false })
    expect(onChange).toHaveBeenNthCalledWith(2, { showTabFolders: true })
  })

  it('mirrors a disabled git-branch switch through aria-checked (#80)', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={{ ...defaultSettings, showTabBranch: false, showTabFolders: true }}
        onChange={() => {}}
        onClose={() => {}}
      />,
    )

    expect(getByTestId('toggle-show-tab-branch')).toHaveAttribute('aria-checked', 'false')
    expect(getByTestId('toggle-show-tab-folders')).toHaveAttribute('aria-checked', 'true')
  })

  // T3 (analytics is invisible to the user — always on, no switch; the HITL
  //   product decision): the dialog must not mention analytics at all.
  it('does not surface any analytics wording', () => {
    const { queryByText } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )

    expect(queryByText(/analytics/i)).toBeNull()
  })

  // --- #77 (v1.6.0): the "Default shell" section -----------------------------
  //
  // The picker is fed by the ShellDetector list (passed in via `shells` — the
  // WorkspaceShell glue runs the probes) plus ALWAYS an "Auto" entry (null =
  // today's backend fallback chain) and a custom-entry text field for any
  // command. Values are what pty_open later receives — verbatim.
  //
  // Fix round 2026-09-09: the control is the SAME dropdown the sidebar "+"
  // button uses (a press-styled button unfolding a .create-dropdown menu of
  // .menu-item rows), not a native <select> — and the dialog scrolls when
  // the window is small.

  const SHELLS = [
    { displayName: 'Bash', launchCommand: '/bin/bash' },
    { displayName: 'Fish', launchCommand: '/usr/bin/fish' },
  ]

  const menuItems = (getByTestId: (id: string) => HTMLElement): string[] =>
    Array.from(getByTestId('shell-picker-menu').querySelectorAll('[role="menuitem"]')).map(
      (o) => o.textContent,
    )

  it('shows the current selection on the picker button, custom field below (#77)', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    // A fresh install is Auto.
    expect(getByTestId('shell-picker')).toHaveTextContent('Auto')
    // The Custom… entry lives in the menu, not on the face of the dialog.
    expect(screen.queryByTestId('shell-custom-input')).toBeNull()
  })

  it('renders the Default shell section even with nothing detected (#77)', () => {
    const { getByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )

    expect(getByTestId('shell-picker')).toHaveTextContent('Auto')
    fireEvent.click(getByTestId('shell-picker'))
    expect(menuItems(getByTestId)).toEqual(['Auto', 'Custom…'])
  })

  it('opens the plus-style dropdown listing Auto and detected shells in order (#77)', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    expect(screen.queryByTestId('shell-picker-menu')).toBeNull()
    fireEvent.click(getByTestId('shell-picker'))
    expect(menuItems(getByTestId)).toEqual(['Auto', 'Bash', 'Fish', 'Custom…'])
  })

  // Fix round 2026-09-11 (HITL): the settings card scrolls (overflow-y: auto),
  // which used to clip the absolute dropdown exactly at the card wall when a
  // long entry (a WSL distro command) widened it. The menu must escape the
  // card: fixed-positioned from the button's box with inline top/left (the
  // same contract as the #78 tab dropdown), an on-screen maxWidth, and the
  // full label riding each row's title tooltip.
  it('positions the menu off the card clip and keeps long labels reachable', () => {
    const longName = '"C:\\Program Files\\WSL\\wsl.exe" -d Ubuntu-22.04'
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        shells={[...SHELLS, { displayName: longName, launchCommand: longName }]}
      />,
    )

    fireEvent.click(getByTestId('shell-picker'))
    const menu = getByTestId('shell-picker-menu')
    expect(menu).toHaveClass('settings-shell-dropdown')
    expect(menu.style.top).not.toBe('')
    expect(menu.style.left).not.toBe('')
    expect(menu.style.maxWidth).not.toBe('')
    const longRow = Array.from(menu.querySelectorAll('[role="menuitem"]')).find(
      (o) => o.textContent === longName,
    )!
    expect(longRow.getAttribute('title')).toBe(longName)
  })

  // Fix round 3 (HITL): the menu is never shrunk to fit the window. When it
  // would cross the window's bottom edge it opens ABOVE the button instead;
  // with room below it drops back under. jsdom has no layout, so the test
  // stubs the button's rect and the menu's offsetHeight.
  it('flips the menu above the button when there is no room below', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )
    const button = getByTestId('shell-picker')
    // Button low in the window: rect {top: 500, bottom: 520}; menu is 300 tall.
    button.getBoundingClientRect = () =>
      ({
        top: 500,
        bottom: 520,
        left: 40,
        right: 140,
        width: 100,
        height: 20,
        x: 40,
        y: 500,
        toJSON: () => {},
      }) as DOMRect
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
      configurable: true,
      value: 300,
    })
    try {
      // 820 tall: below the button there is only 296px (820 - 524) — not
      // enough for 300; above there is 496px → the menu flips above.
      Object.defineProperty(window, 'innerHeight', { configurable: true, value: 820 })
      fireEvent.click(button)
      expect(getByTestId('shell-picker-menu').style.top).toBe('196px')

      // Reopen with room below (900 - 524 = 376 ≥ 300) → back under the button.
      fireEvent.click(button) // toggles the open menu closed
      Object.defineProperty(window, 'innerHeight', { configurable: true, value: 900 })
      fireEvent.click(button)
      expect(getByTestId('shell-picker-menu').style.top).toBe('524px')
    } finally {
      delete (HTMLElement.prototype as { offsetHeight?: unknown }).offsetHeight
      Object.defineProperty(window, 'innerHeight', { configurable: true, value: 768 })
    }
  })

  it('reports a picked shell as defaultShell=launchCommand, verbatim, and closes (#77)', () => {
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={onChange}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    fireEvent.click(getByTestId('shell-picker'))
    fireEvent.click(
      Array.from(getByTestId('shell-picker-menu').querySelectorAll('[role="menuitem"]')).find(
        (o) => o.textContent === 'Fish',
      )!,
    )

    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith({ defaultShell: '/usr/bin/fish' })
    expect(screen.queryByTestId('shell-picker-menu')).toBeNull()
  })

  it('reports the Auto pick as defaultShell=null (#77)', () => {
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={{ ...defaultSettings, defaultShell: '/bin/bash' }}
        onChange={onChange}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    fireEvent.click(getByTestId('shell-picker'))
    fireEvent.click(
      Array.from(getByTestId('shell-picker-menu').querySelectorAll('[role="menuitem"]')).find(
        (o) => o.textContent === 'Auto',
      )!,
    )

    expect(onChange).toHaveBeenCalledWith({ defaultShell: null })
  })

  it('mirrors a saved shell on the button label (#77)', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={{ ...defaultSettings, defaultShell: '/usr/bin/fish' }}
        onChange={() => {}}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    expect(getByTestId('shell-picker')).toHaveTextContent('Fish')
  })

  it('offers a saved custom command in the menu and reflects it (#77)', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={{ ...defaultSettings, defaultShell: 'wsl.exe ~' }}
        onChange={() => {}}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    expect(getByTestId('shell-picker')).toHaveTextContent('wsl.exe ~')
    fireEvent.click(getByTestId('shell-picker'))
    expect(menuItems(getByTestId)).toEqual(['Auto', 'Bash', 'Fish', 'wsl.exe ~', 'Custom…'])
  })

  // The custom entry is a "Custom…" MENU ITEM (user request, fix round 2):
  // picking it opens a small dialog with the command field. Use applies the
  // command trimmed; Cancel (or Escape) closes without changing anything.
  it('opens the Custom dialog from the menu, prefilled with a saved command (#77)', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={{ ...defaultSettings, defaultShell: 'wsl.exe ~' }}
        onChange={() => {}}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    fireEvent.click(getByTestId('shell-picker'))
    fireEvent.click(getByTestId('shell-custom-item'))

    // The command in effect prefills the field — reopening it explains the
    // current selection instead of starting from scratch.
    expect(getByTestId('shell-custom-input')).toHaveValue('wsl.exe ~')
    expect(getByTestId('shell-custom-input')).toHaveFocus()
  })

  it('applies the custom entry verbatim, trimmed, and closes the dialog (#77)', () => {
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={onChange}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    fireEvent.click(getByTestId('shell-picker'))
    fireEvent.click(getByTestId('shell-custom-item'))
    fireEvent.change(getByTestId('shell-custom-input'), {
      target: { value: '  mysh --login  ' },
    })
    fireEvent.click(getByTestId('shell-custom-apply'))

    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith({ defaultShell: 'mysh --login' })
    expect(screen.queryByTestId('shell-custom-dialog')).toBeNull()
  })

  it('an empty custom command does nothing, Cancel closes without changing (#77)', () => {
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={onChange}
        onClose={() => {}}
        shells={SHELLS}
      />,
    )

    fireEvent.click(getByTestId('shell-picker'))
    fireEvent.click(getByTestId('shell-custom-item'))
    fireEvent.change(getByTestId('shell-custom-input'), { target: { value: '   ' } })
    fireEvent.click(getByTestId('shell-custom-apply'))
    expect(onChange).not.toHaveBeenCalled()
    expect(getByTestId('shell-custom-dialog')).toBeInTheDocument()

    fireEvent.click(getByTestId('shell-custom-cancel'))
    expect(onChange).not.toHaveBeenCalled()
    expect(screen.queryByTestId('shell-custom-dialog')).toBeNull()
  })

  it('Escape inside the Custom dialog closes only the dialog (#77)', () => {
    const onClose = vi.fn()
    const onChange = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={onChange}
        onClose={onClose}
        shells={SHELLS}
      />,
    )

    fireEvent.click(getByTestId('shell-picker'))
    fireEvent.click(getByTestId('shell-custom-item'))
    fireEvent.keyDown(window, { key: 'Escape' })

    expect(screen.queryByTestId('shell-custom-dialog')).toBeNull()
    expect(onChange).not.toHaveBeenCalled()
    expect(onClose).not.toHaveBeenCalled()
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

// --- Factory reset (#74) ------------------------------------------------------
//
// Assumptions encoded:
//  - The Reset row renders only when the parent passes onResetAll.
//  - Two deliberate clicks: the first ARMS (label changes, onResetAll NOT
//    called), the second FIRES. A stray press can never wipe the store.
//  - The invoke + relaunch live in WorkspaceShell — this component only
//    reports upward (same invoke-free contract as every other row).
describe('SettingsDialog factory reset (#74)', () => {
  it('hides the Reset row when no onResetAll is given', () => {
    const { queryByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )
    expect(queryByTestId('reset-row')).toBeNull()
  })

  it('the first click only ARMS the button — onResetAll does not fire', () => {
    const onResetAll = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onResetAll={onResetAll}
      />,
    )

    const button = getByTestId('reset-button')
    expect(button.textContent).toMatch(/reset umux/i)
    fireEvent.click(button)

    expect(button.textContent).toMatch(/really reset everything/i)
    expect(onResetAll).not.toHaveBeenCalled()
  })

  it('the second click fires onResetAll exactly once', () => {
    const onResetAll = vi.fn()
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onResetAll={onResetAll}
      />,
    )

    const button = getByTestId('reset-button')
    fireEvent.click(button) // arm
    fireEvent.click(button) // confirm

    expect(onResetAll).toHaveBeenCalledTimes(1)
  })

  // --- umux Storestation (#86, v1.7.0) -------------------------------------

  // The section only renders when the parent wires the toggle (the same
  // optional-prop contract as the updates row): absent handler = absent
  // section, so the OFF state's dialog is byte-identical to v1.6.x's.
  it('hides the Storestation section when no toggle handler is wired', () => {
    const { queryByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )
    expect(queryByTestId('storestation-row')).toBeNull()
    expect(queryByTestId('toggle-storestation')).toBeNull()
  })

  // AC: the section renders — the daemon toggle mirrors the persisted
  // setting (OFF by default) and the row is labeled.
  it('renders the Storestation section with the daemon toggle defaulting OFF', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={() => {}}
        storestationStatus={null}
      />,
    )
    expect(getByTestId('storestation-row')).toBeTruthy()
    expect(getByTestId('toggle-storestation')).toHaveAttribute('aria-checked', 'false')
    expect(getByTestId('toggle-storestation').getAttribute('aria-label')).toMatch(
      /storestation/i,
    )
  })

  // The toggle reports UPWARD (invoke-free component): the click hands the
  // requested next state to the parent, which runs the spawn/stop flow.
  it('reports a daemon-toggle flip to the parent in both directions', () => {
    const onStorestationToggle = vi.fn()
    const { getByTestId, rerender } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={onStorestationToggle}
      />,
    )
    fireEvent.click(getByTestId('toggle-storestation'))
    expect(onStorestationToggle).toHaveBeenCalledWith(true)

    // After the parent persisted ON, the switch mirrors it and a click
    // reports OFF (the stop direction).
    rerender(
      <SettingsDialog
        settings={{
          ...defaultSettings,
          storestation: { daemonEnabled: true, autostartEnabled: false },
        }}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={onStorestationToggle}
      />,
    )
    expect(getByTestId('toggle-storestation')).toHaveAttribute('aria-checked', 'true')
    fireEvent.click(getByTestId('toggle-storestation'))
    expect(onStorestationToggle).toHaveBeenLastCalledWith(false)
  })

  // The status line: running names the daemon's version and session count;
  // stopped says so; no status yet renders nothing.
  it('reflects the live daemon status', () => {
    const { getByTestId, queryByTestId, rerender } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={() => {}}
        storestationStatus={null}
      />,
    )
    expect(queryByTestId('storestation-status')).toBeNull()

    rerender(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={() => {}}
        storestationStatus={{ enabled: true, running: true, version: '1.7.0', sessions: 2 }}
      />,
    )
    const status = getByTestId('storestation-status')
    expect(status.textContent).toMatch(/running/i)
    expect(status.textContent).toContain('1.7.0')
    expect(status.textContent).toContain('2')

    rerender(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={() => {}}
        storestationStatus={{ enabled: true, running: false }}
      />,
    )
    expect(getByTestId('storestation-status').textContent).toMatch(/stopped/i)
  })

  // #89 (v1.7.0 phase 7 — story 108 complete): the section shows ALL THREE
  // controls — daemon toggle, autostart toggle, live status. The autostart
  // toggle mirrors its persisted flag and reports flips upward like the
  // daemon one; without the parent's handler the row is absent (backward
  // compatible with the phase-4 section).
  it('shows all three Storestation controls and reports autostart flips (#89)', () => {
    const onStorestationToggle = vi.fn()
    const onStorestationAutostartToggle = vi.fn()
    const { getByTestId, queryByTestId, rerender } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={onStorestationToggle}
        onStorestationAutostartToggle={onStorestationAutostartToggle}
        storestationStatus={{ enabled: true, running: false }}
      />,
    )
    // Three controls present.
    expect(getByTestId('toggle-storestation')).toBeTruthy()
    expect(getByTestId('toggle-storestation-autostart')).toBeTruthy()
    expect(getByTestId('storestation-status')).toBeTruthy()
    // Autostart starts OFF (opt-in everywhere).
    expect(getByTestId('toggle-storestation-autostart')).toHaveAttribute('aria-checked', 'false')

    // The flip reports upward; the persisted ON state mirrors back.
    fireEvent.click(getByTestId('toggle-storestation-autostart'))
    expect(onStorestationAutostartToggle).toHaveBeenCalledWith(true)
    expect(onStorestationToggle).not.toHaveBeenCalled()

    rerender(
      <SettingsDialog
        settings={{
          ...defaultSettings,
          storestation: { daemonEnabled: false, autostartEnabled: true },
        }}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={onStorestationToggle}
        onStorestationAutostartToggle={onStorestationAutostartToggle}
        storestationStatus={{ enabled: true, running: false }}
      />,
    )
    expect(getByTestId('toggle-storestation-autostart')).toHaveAttribute('aria-checked', 'true')
  })

  it('hides the autostart row when the parent has no handler (phase-4 shape intact)', () => {
    const { getByTestId, queryByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={() => {}}
      />,
    )
    expect(getByTestId('storestation-row')).toBeTruthy()
    expect(queryByTestId('storestation-autostart-row')).toBeNull()
  })
})
