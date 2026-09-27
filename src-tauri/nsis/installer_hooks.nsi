; Custom hooks for the CryptEnv NSIS installer.
; Included by Tauri's generated NSIS script via tauri.conf.json nsis.installerHooks.

; --- PATH management (inlined to avoid relative-path issues at NSIS build time) ---

!include "WordFunc.nsh"

!define PATH_SAFE_CHAR_LIMIT 900

; GetTruePathLenImpl — measures the TRUE length of HKCU\Environment\Path by
; querying the registry API directly (RegQueryValueExW with a NULL buffer,
; the documented Win32 way to ask "how big is this value?" without reading
; it). This is the fix for a critical bug: the previous guard inferred
; truncation from StrLen of whatever ReadRegStr returned — but ReadRegStr
; itself silently truncates any value longer than NSIS_MAX_STRLEN with no
; error signal, so a genuinely huge PATH could come back already truncated
; to something UNDER the guard's threshold, pass the check as "safe", and
; then get written back verbatim — permanently destroying everything past
; the truncation point. This measures the real on-disk size first, before
; any bounded NSIS string variable is ever involved, so the decision to
; proceed is based on ground truth, not on a value that may already be
; mangled.
;
; Output: $8 = true length in UTF-16 characters (rounded up by 1 — a safe
;              overestimate, never an underestimate). 0 if the value is
;              absent.
;         $9 = 1 if the length is known-good. 0 if the registry query
;              itself failed for any reason — callers MUST treat 0 as
;              "unknown, refuse to touch PATH", never as "assume safe".
!macro GetTruePathLenImpl
  StrCpy $8 0
  StrCpy $9 0

  System::Call 'advapi32::RegOpenKeyExW(i 0x80000001, w "Environment", i 0, i 1, *i .r0) i .r1'
  StrCmp $1 0 0 get_true_len_done

  System::Call 'advapi32::RegQueryValueExW(i r0, w "Path", i 0, i 0, i 0, *i 0 r2) i .r1'
  System::Call 'advapi32::RegCloseKey(i r0)'

  StrCmp $1 2 get_true_len_missing
  StrCmp $1 0 get_true_len_ok get_true_len_done

  get_true_len_ok:
    IntOp $8 $2 / 2
    StrCpy $9 1
    Goto get_true_len_done

  get_true_len_missing:
    ; ERROR_FILE_NOT_FOUND — no Path value under HKCU\Environment yet.
    StrCpy $8 0
    StrCpy $9 1

  get_true_len_done:
!macroend

; AddToUserPath — appends $INSTDIR to HKCU\Environment\Path if not already present.
Function AddToUserPath
  Var /GLOBAL PathOld
  Var /GLOBAL PathNew

  !insertmacro GetTruePathLenImpl

  ; Refuse unless the registry query both succeeded AND confirms the true
  ; value is comfortably under NSIS's string cap. $9==0 means "unknown" and
  ; must be treated as unsafe, not as "probably fine".
  StrCmp $9 0 path_too_long
  IntCmp $8 ${PATH_SAFE_CHAR_LIMIT} path_ok path_ok path_too_long

  path_ok:
  ReadRegStr $PathOld HKCU "Environment" "Path"

  ; Belt-and-suspenders: the true length above already guarantees this can't
  ; have been truncated, but re-check the round-tripped value's own length
  ; too before it's ever written anywhere.
  StrLen $R1 $PathOld
  IntCmp $R1 ${PATH_SAFE_CHAR_LIMIT} 0 0 path_too_long

  ; Check if already present as an exact entry: search ";<INSTDIR>;" inside
  ; ";<PATH>;" so a longer path that merely contains $INSTDIR (e.g.
  ; "...\CryptEnv\tools") is not mistaken for our entry. Case-insensitive.
  ${WordFind} ";$PathOld;" ";$INSTDIR;" "E+1{" $R0
  IfErrors 0 already_present

  ; Not present — append.
  StrCmp $PathOld "" 0 +3
    StrCpy $PathNew "$INSTDIR"
    Goto write_path
  StrCpy $PathNew "$PathOld;$INSTDIR"

  write_path:
    WriteRegExpandStr HKCU "Environment" "Path" "$PathNew"
    ; Notify all windows (Explorer, open shells) that the environment changed.
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
    Goto already_present

  path_too_long:
    MessageBox MB_OK|MB_ICONINFORMATION "CryptEnv did NOT modify your PATH because it could not be verified safe to edit (a known Windows installer limitation for very long PATH values).$\r$\n$\r$\nTo run 'crypt-env' from any terminal, add this folder to your PATH manually:$\r$\n$\r$\n$INSTDIR"

  already_present:
FunctionEnd

; RemoveFromUserPath — removes $INSTDIR from HKCU\Environment\Path.
Function un.RemoveFromUserPath
  Var /GLOBAL UnPathOld
  Var /GLOBAL UnPathNew

  !insertmacro GetTruePathLenImpl

  ; Same reasoning as AddToUserPath: refuse on anything but a confirmed-safe
  ; true length. Leaving a stale PATH entry behind on uninstall is harmless;
  ; destroying the tail of PATH is not.
  StrCmp $9 0 un_path_done
  IntCmp $8 ${PATH_SAFE_CHAR_LIMIT} un_path_ok un_path_ok un_path_done

  un_path_ok:
  ReadRegStr $UnPathOld HKCU "Environment" "Path"

  StrLen $R1 $UnPathOld
  IntCmp $R1 ${PATH_SAFE_CHAR_LIMIT} 0 0 un_path_done

  ; Remove exactly the "$INSTDIR" entry (every occurrence), never a longer
  ; path that merely contains it: replace ";<INSTDIR>;" inside ";<PATH>;".
  ; Run twice so adjacent duplicates (";X;X;") — whose shared separator is
  ; consumed by the first match — are also removed.
  ${WordReplace} ";$UnPathOld;" ";$INSTDIR;" ";" "+" $UnPathNew
  ${WordReplace} "$UnPathNew" ";$INSTDIR;" ";" "+" $R0
  StrCpy $UnPathNew $R0

  ; Clean up any double semicolons left behind.
  ${WordReplace} "$UnPathNew" ";;" ";" "+" $R0
  StrCpy $UnPathNew $R0

  ; Trim leading/trailing semicolons.
  StrCpy $R0 $UnPathNew 1
  StrCmp $R0 ";" 0 +2
    StrCpy $UnPathNew $UnPathNew "" 1
  StrLen $R0 $UnPathNew
  IntOp $R0 $R0 - 1
  StrCpy $R0 $UnPathNew 1 $R0
  StrCmp $R0 ";" 0 +2
    StrCpy $UnPathNew $UnPathNew -1

  WriteRegExpandStr HKCU "Environment" "Path" "$UnPathNew"
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000

  un_path_done:
FunctionEnd

; --- Installer sections ---

; Called by Tauri after installation completes.
Section "PATH Registration" SecPath
  Call AddToUserPath
SectionEnd

; Called by Tauri's uninstaller.
Section "un.PATH Cleanup" SecUnPath
  Call un.RemoveFromUserPath
SectionEnd
