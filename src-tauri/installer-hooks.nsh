; installer-hooks.nsh — issue #64: put the umux CLI on the user's PATH.
;
; The installer drops the CLI (umux.exe) into $INSTDIR alongside the app
; (umux-app.exe). Tauri runs these macros at the marked points of its NSIS
; script; everything here is BASE NSIS only — no plugins, no StrFunc.
; (First attempt used StrStr, which lives in StrFunc.nsh and is NOT part of
; the base instruction set — makensis refused to compile the hook.)
;
; We write the USER path (HKCU\Environment\Path), not the system one:
;   - no admin rights needed (the Tauri NSIS default install is per-user),
;   - a broken PATH here can only affect this user.
; After writing we broadcast WM_SETTINGCHANGE so EXPLORER reloads the
; environment — terminals opened AFTER the install pick `umux` up with no
; reboot and no logoff. Terminals that were already open keep their old
; PATH (Windows limitation, documented in the README).
;
; The substring search below is a plain base-instruction loop: slide a
; window of len($INSTDIR) across the PATH one character at a time. PATH is
; short; O(n·m) is irrelevant here. Registers used: R0 (value under work),
; R1 (entry length), R2 (PATH length), R3 (index), R4 (window), R5..R9
; (scratch for the splice). Labels are prefixed per macro — NSIS labels are
; script-global, and both macros land in the same script.

!define UMUX_HWND_BROADCAST 0xFFFF
!define UMUX_WM_SETTINGCHANGE 0x001A

!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $R0 HKCU "Environment" "Path"
  StrLen $R1 "$INSTDIR"
  StrLen $R2 "$R0"
  StrCpy $R3 0
install_find_loop:
    ; index >= PATH length → the directory is NOT on PATH yet
    IntCmp $R3 "$R2" install_not_found install_find_body install_not_found
install_find_body:
    StrCpy $R4 "$R0" "$R1" "$R3"
    StrCmp "$R4" "$INSTDIR" 0 install_find_next
      Goto done_install            ; already on PATH (repair/reinstall)
install_find_next:
    IntOp $R3 "$R3" + 1
    Goto install_find_loop
install_not_found:
    StrCmp "$R0" "" install_append
      StrCpy $R0 "$R0;"
install_append:
    StrCpy $R0 "$R0$INSTDIR"
    WriteRegStr HKCU "Environment" "Path" "$R0"
    SendMessage ${UMUX_HWND_BROADCAST} ${UMUX_WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=2000
done_install:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ReadRegStr $R0 HKCU "Environment" "Path"
  StrCmp "$R0" "" done_uninstall
  StrLen $R1 "$INSTDIR"
  StrLen $R2 "$R0"
  StrCpy $R3 0
uninstall_find_loop:
    IntCmp $R3 "$R2" done_uninstall uninstall_find_body done_uninstall
uninstall_find_body:
    StrCpy $R4 "$R0" "$R1" "$R3"
    StrCmp "$R4" "$INSTDIR" 0 uninstall_find_next
      Goto uninstall_splice        ; found at index R3
uninstall_find_next:
    IntOp $R3 "$R3" + 1
    Goto uninstall_find_loop
uninstall_splice:
    ; new PATH = head (before the entry) + tail (after it), with exactly one
    ; ';' at the junction when both sides survive.
    StrCpy $R5 "$R0" "$R3" 0       ; head
    IntOp $R6 "$R3" + "$R1"
    StrCpy $R7 "$R0" "" "$R6"      ; tail
    StrCpy $R8 "$R7" 1 0
    StrCmp "$R8" ";" 0 uninstall_head_check
      StrCpy $R7 "$R7" "" 1        ; tail started with ';' — drop it
uninstall_head_check:
    StrCmp "$R5" "" 0 uninstall_tail_check
      StrCpy $R0 "$R7"             ; no head → just the tail
      Goto uninstall_write
uninstall_tail_check:
    StrCmp "$R7" "" 0 uninstall_glue
      StrCpy $R0 "$R5"             ; no tail → just the head
      Goto uninstall_write
uninstall_glue:
      StrCpy $R0 "$R5;$R7"
uninstall_write:
    StrCmp "$R0" "" 0 uninstall_store
      DeleteRegValue HKCU "Environment" "Path"
      Goto done_uninstall
uninstall_store:
      WriteRegStr HKCU "Environment" "Path" "$R0"
      SendMessage ${UMUX_HWND_BROADCAST} ${UMUX_WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=2000
done_uninstall:
!macroend
