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
  it('hides the Reset section when no onResetAll is given', () => {
    const { queryByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )
    expect(queryByTestId('reset-open')).toBeNull()
    expect(queryByTestId('reset-button')).toBeNull()
  })

  // 2026-10-02 rework: the entry button OPENS the reset screen; the confirm
  // flow itself lives there, unchanged (arm on first click, fire on second).
  it('the entry button opens a dedicated reset screen with the confirm inside', () => {
    const onResetAll = vi.fn()
    const { getByTestId, queryByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onResetAll={onResetAll}
      />,
    )
    expect(queryByTestId('reset-button')).toBeNull()

    fireEvent.click(getByTestId('reset-open'))

    expect(getByTestId('settings-back')).toBeTruthy()
    expect(getByTestId('reset-row')).toBeTruthy()
    expect(getByTestId('reset-button')).toBeTruthy()
    // The reset screen REPLACES the settings page too.
    expect(queryByTestId('toggle-notifications')).toBeNull()

    fireEvent.click(getByTestId('settings-back'))
    expect(queryByTestId('reset-button')).toBeNull()
    expect(getByTestId('toggle-notifications')).toBeTruthy()
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

    fireEvent.click(getByTestId('reset-open'))
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

    fireEvent.click(getByTestId('reset-open'))
    const button = getByTestId('reset-button')
    fireEvent.click(button) // arm
    fireEvent.click(button) // confirm

    expect(onResetAll).toHaveBeenCalledTimes(1)
  })

  // --- umux Storestation (#86, v1.7.0) -------------------------------------

  // The Storestation section only renders when the parent wires the toggle
  // (the same optional-prop contract as the updates row): absent handler =
  // absent entry button AND absent sub-view, so the OFF state's dialog is
  // byte-identical to v1.6.x's.
  it('hides the Storestation section when no toggle handler is wired', () => {
    const { queryByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )
    expect(queryByTestId('storestation-open')).toBeNull()
    expect(queryByTestId('toggle-storestation')).toBeNull()
  })

  // 2026-10-02 rework: the main settings page keeps only the entry BUTTON;
  // the switches live in the dedicated sub-view it opens. Opening shows the
  // Back button and the daemon toggle mirroring the persisted setting (OFF
  // by default).
  it('opens a dedicated Storestation view with the daemon toggle defaulting OFF', () => {
    const { getByTestId, queryByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={() => {}}
        storestationStatus={null}
      />,
    )
    expect(getByTestId('storestation-open')).toBeTruthy()
    expect(queryByTestId('toggle-storestation')).toBeNull()

    fireEvent.click(getByTestId('storestation-open'))

    expect(getByTestId('settings-back')).toBeTruthy()
    expect(getByTestId('toggle-storestation')).toHaveAttribute('aria-checked', 'false')
    expect(getByTestId('toggle-storestation').getAttribute('aria-label')).toMatch(
      /storestation/i,
    )
    // The sub-view REPLACES the settings page: not one main-page control
    // survives (2026-10-02 rework round 2, Adam).
    expect(queryByTestId('toggle-notifications')).toBeNull()
    expect(queryByTestId('reset-button')).toBeNull()
    expect(queryByTestId('update-check')).toBeNull()
    // …but the settings.json footnote is ALWAYS the last line (round 3).
    expect(document.querySelector('.settings-footnote')).toBeTruthy()
  })

  // Back and Escape both return ONE level (sub-view → main page) without
  // closing the dialog; on the main page Escape closes as before.
  it('back and Escape return from the Storestation view without closing', () => {
    const onClose = vi.fn()
    const { getByTestId, queryByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={onClose}
        onStorestationToggle={() => {}}
      />,
    )
    fireEvent.click(getByTestId('storestation-open'))
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).not.toHaveBeenCalled()
    expect(queryByTestId('settings-back')).toBeNull()

    fireEvent.click(getByTestId('storestation-open'))
    fireEvent.click(getByTestId('settings-back'))
    expect(onClose).not.toHaveBeenCalled()
    expect(queryByTestId('toggle-storestation')).toBeNull()

    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
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
    fireEvent.click(getByTestId('storestation-open'))
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

  // The status line (sub-view only): running names the daemon's version and
  // session count; stopped says so; no status yet renders nothing.
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
    fireEvent.click(getByTestId('storestation-open'))
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

  // #89 (v1.7.0 phase 7 — story 108 complete): the sub-view shows ALL THREE
  // controls — daemon toggle, autostart toggle, live status. The autostart
  // toggle mirrors its persisted flag and reports flips upward like the
  // daemon one; without the parent's handler the row is absent (backward
  // compatible with the phase-4 section).
  it('shows all three Storestation controls and reports autostart flips (#89)', () => {
    const onStorestationToggle = vi.fn()
    const onStorestationAutostartToggle = vi.fn()
    const { getByTestId, rerender } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onStorestationToggle={onStorestationToggle}
        onStorestationAutostartToggle={onStorestationAutostartToggle}
        storestationStatus={{ enabled: true, running: false }}
      />,
    )
    fireEvent.click(getByTestId('storestation-open'))
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
    expect(getByTestId('storestation-open')).toBeTruthy()

    fireEvent.click(getByTestId('storestation-open'))

    expect(getByTestId('toggle-storestation')).toBeTruthy()
    expect(queryByTestId('storestation-autostart-row')).toBeNull()
  })

  // --- umux Core (Always-On device, #94, v1.7.5 phase 2) -------------------
  //
  // Assumptions (state-before-RED): the Core switch has NO settings.json
  // entry — the daemon owns the flag — so the component's contract is
  // mirror-and-report: `coreStatus` IS the daemon's view (enabled/held/
  // instruction) plus the parent's `error` from a failed op; the click
  // reports upward; nothing renders from local state.

  // The same optional-prop contract as Storestation: no wired handler = no
  // section at all, the dialog is byte-identical to the pre-Core tree.
  it('hides the Core section when no toggle handler is wired', () => {
    const { queryByTestId } = render(
      <SettingsDialog settings={defaultSettings} onChange={() => {}} onClose={() => {}} />,
    )
    expect(queryByTestId('core-open')).toBeNull()
    expect(queryByTestId('toggle-core')).toBeNull()
  })

  // AC1: the switch is OFF by default and MIRRORS the daemon-reported
  // state — with a daemon reporting enabled, the freshly opened view shows
  // it ON. The entry button reads "Core (Always-On device)".
  it('opens the Core view; the switch mirrors the daemon-reported state, OFF by default', () => {
    const { getByTestId, queryByTestId, rerender } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={() => {}}
        coreStatus={{ enabled: false, held: false, instruction: null, error: null }}
      />,
    )
    expect(getByTestId('core-open').textContent).toMatch(/umux Core \(Always-On\)/)
    expect(queryByTestId('toggle-core')).toBeNull()

    fireEvent.click(getByTestId('core-open'))

    expect(getByTestId('settings-back')).toBeTruthy()
    expect(getByTestId('toggle-core').getAttribute('aria-label')).toMatch(
      /umux Core \(Always-On\)/,
    )
    expect(getByTestId('toggle-core')).toHaveAttribute('aria-checked', 'false')

    // The parent re-syncs the mirror from the daemon (its only source of
    // truth): an enabled daemon shows the switch ON.
    rerender(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={() => {}}
        coreStatus={{ enabled: true, held: true, instruction: null, error: null }}
      />,
    )
    expect(getByTestId('toggle-core')).toHaveAttribute('aria-checked', 'true')
  })

  // The view replaces the whole settings page; the footnote is ALWAYS the
  // last line inside it (2026-10-02 rework rule, carried to Core).
  it('the Core view replaces the main page and keeps the footnote last', () => {
    const { getByTestId, queryByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={() => {}}
        coreStatus={{ enabled: false, held: false, instruction: null, error: null }}
      />,
    )
    fireEvent.click(getByTestId('core-open'))

    expect(queryByTestId('toggle-notifications')).toBeNull()
    expect(queryByTestId('toggle-storestation')).toBeNull()
    expect(queryByTestId('reset-button')).toBeNull()
    expect(document.querySelector('.settings-footnote')).toBeTruthy()
    // One Back button — the Core view's own (Storestation's lives in ITS
    // view; they never render together).
    expect(document.querySelectorAll('[data-testid="settings-back"]').length).toBe(1)
  })

  // AC2 (component side of the pessimistic flow): the click reports the
  // requested next state upward — the parent spawns the daemon when absent
  // and sends core.set; the switch only ever MIRRORS the daemon's answer.
  it('reports a core-toggle flip to the parent in both directions', () => {
    const onCoreToggle = vi.fn()
    const { getByTestId, rerender } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={onCoreToggle}
        coreStatus={{ enabled: false, held: false, instruction: null, error: null }}
      />,
    )
    fireEvent.click(getByTestId('core-open'))
    fireEvent.click(getByTestId('toggle-core'))
    expect(onCoreToggle).toHaveBeenCalledWith(true)

    // After the daemon answered ON, a click reports the OFF direction.
    rerender(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={onCoreToggle}
        coreStatus={{ enabled: true, held: true, instruction: null, error: null }}
      />,
    )
    expect(getByTestId('toggle-core')).toHaveAttribute('aria-checked', 'true')
    fireEvent.click(getByTestId('toggle-core'))
    expect(onCoreToggle).toHaveBeenLastCalledWith(false)
  })

  // AC3: a failed op leaves the persisted (daemon-reported) state UNTOUCHED
  // and surfaces the error — the switch keeps showing the daemon's truth,
  // the error renders in the view, and nothing hides silently.
  it('a failed op renders the error and leaves the mirrored state untouched', () => {
    const { getByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={() => {}}
        coreStatus={{
          enabled: false,
          held: false,
          instruction: null,
          error: 'umux Storestation is not running.',
        }}
      />,
    )
    fireEvent.click(getByTestId('core-open'))

    const error = getByTestId('core-error')
    expect(error.textContent).toContain('umux Storestation is not running.')
    expect(getByTestId('toggle-core')).toHaveAttribute('aria-checked', 'false')
  })

  // AC4: the platform instruction renders when present (the battery caveat
  // on macOS) and the line is ABSENT when there is nothing to instruct
  // about — the screen hides nothing, and invents nothing.
  it('renders the platform instruction only when the daemon reports one', () => {
    const { getByTestId, queryByTestId, rerender } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={() => {}}
        coreStatus={{
          enabled: true,
          held: true,
          instruction: 'Running on battery power: macOS can still force sleep.',
          error: null,
        }}
      />,
    )
    fireEvent.click(getByTestId('core-open'))
    expect(getByTestId('core-instruction').textContent).toContain(
      'Running on battery power',
    )

    rerender(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={() => {}}
        coreStatus={{ enabled: true, held: true, instruction: null, error: null }}
      />,
    )
    expect(queryByTestId('core-instruction')).toBeNull()
  })

  // The daemon status line: a running daemon names itself and says whether
  // the sleep block is held; a stopped daemon says so.
  it('reflects the daemon status in the Core view', () => {
    const { getByTestId, rerender } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={() => {}}
        coreStatus={{ enabled: true, held: true, instruction: null, error: null }}
        storestationStatus={{ enabled: true, running: true, version: '1.7.0' }}
      />,
    )
    fireEvent.click(getByTestId('core-open'))
    const status = getByTestId('core-status')
    expect(status.textContent).toMatch(/running/i)
    expect(status.textContent).toContain('1.7.0')
    expect(status.textContent).toMatch(/sleep prevention held\./i)

    rerender(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={() => {}}
        onCoreToggle={() => {}}
        coreStatus={{ enabled: true, held: false, instruction: null, error: null }}
        storestationStatus={{ enabled: true, running: false }}
      />,
    )
    expect(getByTestId('core-status').textContent).toMatch(/daemon stopped\./i)
  })

  // Escape returns ONE level (Core view → main page) without closing the
  // dialog — the one dismissal reflex per level, same as Storestation.
  it('Escape returns from the Core view to the main page without closing', () => {
    const onClose = vi.fn()
    const { getByTestId, queryByTestId } = render(
      <SettingsDialog
        settings={defaultSettings}
        onChange={() => {}}
        onClose={onClose}
        onCoreToggle={() => {}}
        coreStatus={{ enabled: false, held: false, instruction: null, error: null }}
      />,
    )
    fireEvent.click(getByTestId('core-open'))
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).not.toHaveBeenCalled()
    expect(queryByTestId('toggle-core')).toBeNull()

    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  // --- The Windows lid caveat (issue #95 follow-up, 2026-10-04) ------------
  //
  // Assumptions (state-before-RED): on Windows the HELD instruction IS the
  // lid caveat — a long how-to that drowns the Settings screen — so the
  // screen renders a SHORT note plus a help button, and the full step list
  // lives one level deeper. Every OTHER instruction still renders in full:
  // Windows refusals (held:false) and non-Windows caveats (the macOS
  // battery note) hide nothing.

  const setPlatform = (value: string) => {
    Object.defineProperty(window.navigator, 'platform', { value, configurable: true })
  }
  const restorePlatform = () => {
    // The jsdom value is a prototype getter; deleting the own property
    // restores it.
    delete (window.navigator as { platform?: string }).platform
  }
  const LID_INSTRUCTION =
    'umux Core is keeping this machine awake while idle. If this device has a closing lid: the lid-close action still follows your Windows power plan — to stay awake with the lid closed, set it to "Do nothing" in Power Options (Control Panel → "Choose what closing the lid does").'

  // On Windows the held instruction is the lid caveat: SHORT note + button,
  // never the raw paragraph; the button opens the step-by-step view.
  it('on Windows, a held lid caveat renders short with a help view, not the raw string', () => {
    setPlatform('Win32')
    try {
      const { getByTestId, queryByTestId } = render(
        <SettingsDialog
          settings={defaultSettings}
          onChange={() => {}}
          onClose={() => {}}
          onCoreToggle={() => {}}
          coreStatus={{ enabled: true, held: true, instruction: LID_INSTRUCTION, error: null }}
        />,
      )
      fireEvent.click(getByTestId('core-open'))

      // The raw how-to paragraph is GONE from the screen; a short note and
      // the button take its place.
      expect(queryByTestId('core-instruction')).toBeNull()
      expect(getByTestId('core-lid-note').textContent).toMatch(/power plan/i)
      expect(getByTestId('core-lid-note').textContent).not.toContain('Control Panel')

      // The help view: one level deeper, with the actual steps; Back
      // returns to the Core view (the switch is there again).
      fireEvent.click(getByTestId('core-lid-help'))
      const help = getByTestId('core-lid-help-view')
      expect(help.textContent).toContain('Choose what closing the lid does')
      expect(help.textContent).toContain('Do nothing')
      fireEvent.click(getByTestId('settings-back'))
      expect(queryByTestId('core-lid-help-view')).toBeNull()
      expect(queryByTestId('toggle-core')).not.toBeNull()
    } finally {
      restorePlatform()
    }
  })

  // A Windows refusal (held:false) is an error, not a how-to — it still
  // renders in full, with no help button to dilute it.
  it('on Windows, a not-held instruction still renders in full (refusals hide nothing)', () => {
    setPlatform('Win32')
    try {
      const { getByTestId, queryByTestId } = render(
        <SettingsDialog
          settings={defaultSettings}
          onChange={() => {}}
          onClose={() => {}}
          onCoreToggle={() => {}}
          coreStatus={{
            enabled: true,
            held: false,
            instruction: 'Windows refused the Always-On hold (SetThreadExecutionState failed).',
            error: null,
          }}
        />,
      )
      fireEvent.click(getByTestId('core-open'))
      expect(getByTestId('core-instruction').textContent).toContain('refused')
      expect(queryByTestId('core-lid-help')).toBeNull()
    } finally {
      restorePlatform()
    }
  })

  // Off Windows (the macOS battery caveat in this test) the instruction
  // renders in full as before — the Windows help flow is Windows-only.
  it('off Windows, the instruction renders in full with no help button', () => {
    setPlatform('MacIntel')
    try {
      const { getByTestId, queryByTestId } = render(
        <SettingsDialog
          settings={defaultSettings}
          onChange={() => {}}
          onClose={() => {}}
          onCoreToggle={() => {}}
          coreStatus={{
            enabled: true,
            held: true,
            instruction: 'Running on battery power: macOS can still force sleep.',
            error: null,
          }}
        />,
      )
      fireEvent.click(getByTestId('core-open'))
      expect(getByTestId('core-instruction').textContent).toContain('Running on battery power')
      expect(queryByTestId('core-lid-help')).toBeNull()
    } finally {
      restorePlatform()
    }
  })
})
