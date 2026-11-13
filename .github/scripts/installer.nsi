; ============================================================================
;  K music installer (NSIS, Unicode)
;
;  Build (run from repo root):
;    makensis /DPRODUCT_VERSION=0.12.5 /DDIST_DIR=dist .github/scripts/installer.nsi
;  Output:
;    k-music-setup-<version>-x64.exe   (in repo root)
;
;  Design notes (learned from project facts - do not remove):
;    1) ASCII-only comments/UI text. The app writes runtime config
;       (kqk-app.toml, library.db, .guest, kqk-instance.port,
;       kqk-window.state, app.log) into its OWN exe directory, so we install
;       per-user under $LOCALAPPDATA\Programs\K music (no UAC, writable).
;    2) Runtime config files are EXCLUDED from the file copy so a reinstall /
;       upgrade never overwrites the user's existing data.
;    3) WebView2 runtime is detected via the official EdgeUpdate client GUID;
;       if missing, the Evergreen bootstrapper is downloaded (system TLS via
;       PowerShell, follows fwlink redirect) and run silently. Failure is
;       non-fatal - the app itself also prompts at first launch.
; ============================================================================

!ifndef PRODUCT_VERSION
  !define PRODUCT_VERSION "0.0.0"
!endif

!define PRODUCT_NAME "K music"
!define PUBLISHER "KKitStudio"
!define WEBVIEW2_GUID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!define WEBVIEW2_BOOTSTRAPPER "https://go.microsoft.com/fwlink/p/?LinkId=2124703"

!include "MUI2.nsh"

Name "${PRODUCT_NAME}"
OutFile "k-music-setup-${PRODUCT_VERSION}-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\K music"
InstallDirRegKey HKCU "Software\KKitStudio\K music" "InstallDir"
RequestExecutionLevel user

VIProductVersion "${PRODUCT_VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileVersion" "${PRODUCT_VERSION}.0"
VIAddVersionKey "ProductVersion" "${PRODUCT_VERSION}"

!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!ifndef DIST_DIR
  !define DIST_DIR "dist"
!endif

Section "Install" SecInstall
  SetOutPath "$INSTDIR"

  ; Install everything EXCEPT runtime-generated user config (preserve on upgrade).
  SetOverwrite on
  File /r /x "kqk-app.toml" /x "library.db" /x ".guest" /x "kqk-instance.port" /x "kqk-window.state" /x "app.log" "${DIST_DIR}\*"

  ; Start menu + desktop shortcuts.
  CreateDirectory "$SMPROGRAMS\${PRODUCT_NAME}"
  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk" "$INSTDIR\k_music.exe"
  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}\Uninstall.lnk" "$INSTDIR\uninstall.exe"
  CreateShortcut "$DESKTOP\${PRODUCT_NAME}.lnk" "$INSTDIR\k_music.exe"

  ; Uninstaller + registry entries (per-user, HKCU - no admin needed).
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "Publisher" "${PUBLISHER}"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}" "NoRepair" 1

  ; Remember install dir for the directory page on reinstall.
  WriteRegStr HKCU "Software\KKitStudio\K music" "InstallDir" "$INSTDIR"

  ; Best-effort: ensure WebView2 runtime (silent, non-fatal).
  Call EnsureWebView2
SectionEnd

Function EnsureWebView2
  ClearErrors
  ReadRegStr $0 HKLM "Software\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2_GUID}" "pv"
  IfErrors 0 WebView2Done
  ReadRegStr $0 HKCU "Software\Microsoft\EdgeUpdate\Clients\${WEBVIEW2_GUID}" "pv"
  IfErrors 0 WebView2Done

  ; Missing: download Evergreen bootstrapper (PowerShell follows the fwlink
  ; redirect and uses system TLS) then run it silently.
  StrCpy $1 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
  StrCpy $2 "powershell -NoProfile -WindowStyle Hidden -Command $\"Invoke-WebRequest -Uri $\"${WEBVIEW2_BOOTSTRAPPER}$\" -OutFile $\"$1$\" -UseBasicParsing$\""
  ExecWait $2
  IfFileExists $1 0 WebView2Done
  ExecWait '"$1" /silent /install'
  Delete $1
WebView2Done:
FunctionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\Uninstall.lnk"
  RMDir "$SMPROGRAMS\${PRODUCT_NAME}"
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"
  RMDir /r "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"
  DeleteRegKey HKCU "Software\KKitStudio\K music"
SectionEnd
