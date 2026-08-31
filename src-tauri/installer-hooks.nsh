; installer-hooks.nsh — issue #64: put the umux CLI on the user's PATH.
;
; The installer drops the CLI (umux.exe) into $INSTDIR alongside the app
; (umux-app.exe). Tauri runs these macros at the marked points of its NSIS
; script; everything here is plain NSIS — no extra plugins (zero-cost
; policy, and the Tauri-bundled NSIS carries no EnVar plugin).
;
; We write the USER path (HKCU\Environment\Path), not the system one:
;   - no admin rights needed (the Tauri NSIS default install is per-user),
;   - a broken PATH here can only affect this user.
; After writing we broadcast WM_SETTINGCHANGE so EXPLORER reloads the
; environment — terminals opened AFTER the install pick `umux` up with no
; reboot and no logoff. Terminals that were already open keep their old
; PATH (Windows limitation, documented in the README).
;
; Style note: explicit labels everywhere. Relative jumps (+2/+3) in NSIS
; are counted from the NEXT instruction and are how subtle installer bugs
; are born; this hook is small enough to afford names.

!define UMUX_HWND_BROADCAST 0xFFFF
!define UMUX_WM_SETTINGCHANGE 0x001A

!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $R0 HKCU "Environment" "Path"
  ; Skip when the directory is already on PATH (repair/reinstall over
  ; itself — appending twice would grow the value on every run).
  StrStr $R1 "$R0" "$INSTDIR"
  StrCmp "$R1" "" 0 done_install
  StrCmp "$R0" "" append_path
    StrCpy $R0 "$R0;"
append_path:
    StrCpy $R0 "$R0$INSTDIR"
    WriteRegStr HKCU "Environment" "Path" "$R0"
    SendMessage ${UMUX_HWND_BROADCAST} ${UMUX_WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=2000
done_install:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ReadRegStr $R0 HKCU "Environment" "Path"
  StrCmp "$R0" "" done_uninstall
  StrLen $R2 "$INSTDIR"
  ; Case A: PATH begins with $INSTDIR — the head of the value is ours.
  StrCpy $R3 "$R0" "$R2" 0
  StrCmp "$R3" "$INSTDIR" 0 case_b
    StrCpy $R4 "$R0" "" "$R2"      ; what follows $INSTDIR
    StrCmp "$R4" "" remove_value   ; $INSTDIR was the ONLY entry
    StrCmp "$R4" ";" remove_value  ; PATH was exactly "$INSTDIR;"
    StrCpy $R0 "$R4" "" 1          ; drop the ';' that separated it
    Goto write_uninstall
  ; Case B: ";$INSTDIR" appears later (middle or end) — glue the parts
  ; around the match back together.
case_b:
    StrCpy $R4 ";$INSTDIR"
    StrLen $R5 "$R4"
    StrStr $R6 "$R0" "$R4"
    StrCmp "$R6" "" done_uninstall ; not on PATH — nothing to do
    StrLen $R7 "$R0"
    StrLen $R8 "$R6"
    IntOp $R7 "$R7" - "$R8"        ; prefix length
    StrCpy $R9 "$R0" "$R7" 0       ; prefix
    StrCpy $R6 "$R6" "" "$R5"      ; suffix after ";$INSTDIR"
    ; Glue prefix and suffix back — re-inserting the ';' only when a suffix
    ; exists, so the tail entry ("a;$INSTDIR" → "a") never leaves a stray ';'
    ; and middle removal keeps its separator ("a;$INSTDIR;b" → "a;b").
    StrCmp "$R6" "" 0 glue_semi
      StrCpy $R0 "$R9"
      Goto write_uninstall
glue_semi:
      StrCpy $R0 "$R9;$R6"
write_uninstall:
    StrCmp "$R0" "" 0 store_value
      DeleteRegValue HKCU "Environment" "Path"
      Goto done_uninstall
store_value:
      WriteRegStr HKCU "Environment" "Path" "$R0"
      SendMessage ${UMUX_HWND_BROADCAST} ${UMUX_WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=2000
done_uninstall:
!macroend
