; Lyre native (Rune) installer. Built by the release workflow with:
;   makensis /DVERSION=<ver> /DBINARY=<path to lyre-rune.exe> /DOUTFILE=<out.exe> lyre.nsi
; Per-user install (no UAC): the folder-handler registration the app writes is
; per-user too, so everything stays in HKCU and %LOCALAPPDATA%.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef BINARY
  !define BINARY "lyre-rune.exe"
!endif
!ifndef OUTFILE
  !define OUTFILE "Lyre-setup.exe"
!endif
; Paths are relative to the directory makensis is invoked from (the repo root).
!ifndef ICON
  !define ICON "src-tauri/icons/icon.ico"
!endif

!define WM_CLOSE 0x0010
!define PRODUCT "Lyre"
!define REGKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT}"

Name "${PRODUCT}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT}"
InstallDirRegKey HKCU "${REGKEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  ; Ask a running Lyre to close so its executable is not locked when replaced.
  FindWindow $0 "" "${PRODUCT}"
  ${If} $0 <> 0
    SendMessage $0 ${WM_CLOSE} 0 0
    Sleep 900
  ${EndIf}

  SetOutPath "$INSTDIR"
  File /oname=Lyre.exe "${BINARY}"

  SetShellVarContext current
  CreateDirectory "$SMPROGRAMS\${PRODUCT}"
  CreateShortcut "$SMPROGRAMS\${PRODUCT}\${PRODUCT}.lnk" "$INSTDIR\Lyre.exe"
  CreateShortcut "$SMPROGRAMS\${PRODUCT}\Uninstall.lnk" "$INSTDIR\uninstall.exe"
  CreateShortcut "$DESKTOP\${PRODUCT}.lnk" "$INSTDIR\Lyre.exe"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${REGKEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${REGKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${REGKEY}" "Publisher" "ItsPotatoTime"
  WriteRegStr HKCU "${REGKEY}" "DisplayIcon" "$INSTDIR\Lyre.exe"
  WriteRegStr HKCU "${REGKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${REGKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegDWORD HKCU "${REGKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${REGKEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$DESKTOP\${PRODUCT}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT}\${PRODUCT}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT}\Uninstall.lnk"
  RMDir "$SMPROGRAMS\${PRODUCT}"
  Delete "$INSTDIR\Lyre.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${REGKEY}"
SectionEnd