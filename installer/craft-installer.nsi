; Craft app per-user installer (no admin rights, no UAC prompt).
; VENDORED from mcp-central-docs\templates\craft-app (identical in every craft repo).
; scripts\release.ps1 passes everything as defines:
;   PRODUCT_NAME (display), PRODUCT_ID (repo / data folder name), PRODUCT_VERSION, APP_EXE,
;   CLI_EXE (optional), BIN_DIR (cargo release dir), SRC_DIR (repo root), OUT_FILE, WEB_SITE.
; Installs to %LOCALAPPDATA%\Programs\<PRODUCT_NAME>. App data lives elsewhere
; (%LOCALAPPDATA%\<PRODUCT_ID>), so uninstalling never deletes the user's data.

!ifndef PRODUCT_NAME
  !error "pass /DPRODUCT_NAME=..."
!endif
!ifndef PRODUCT_ID
  !error "pass /DPRODUCT_ID=..."
!endif
!ifndef PRODUCT_VERSION
  !error "pass /DPRODUCT_VERSION=x.y.z"
!endif
!ifndef APP_EXE
  !error "pass /DAPP_EXE=<name>.exe"
!endif
!ifndef BIN_DIR
  !error "pass /DBIN_DIR=<cargo target>\release"
!endif
!ifndef SRC_DIR
  !error "pass /DSRC_DIR=<repo root>"
!endif
!ifndef OUT_FILE
  !define OUT_FILE "${SRC_DIR}\dist\${PRODUCT_ID}-${PRODUCT_VERSION}-setup.exe"
!endif
!ifndef WEB_SITE
  !define WEB_SITE "https://github.com/sandraschi/${PRODUCT_ID}"
!endif

!define DIR_REGKEY "Software\Microsoft\Windows\CurrentVersion\App Paths\${APP_EXE}"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"

Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT_NAME}"
InstallDirRegKey HKCU "${DIR_REGKEY}" ""
ShowInstDetails show
ShowUnInstDetails show

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section "${PRODUCT_NAME}" SEC01
  SetOutPath "$INSTDIR"
  SetOverwrite on
  ; Missing binaries are an error: a half-empty installer is worse than none.
  File "${BIN_DIR}\${APP_EXE}"
!ifdef CLI_EXE
  File "${BIN_DIR}\${CLI_EXE}"
!endif
  File /nonfatal "${SRC_DIR}\README.md"
  File /nonfatal "${SRC_DIR}\HELP.md"
  File /nonfatal "${SRC_DIR}\CHANGELOG.md"

  CreateDirectory "$SMPROGRAMS\${PRODUCT_NAME}"
  CreateShortCut "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}"
  CreateShortCut "$SMPROGRAMS\${PRODUCT_NAME}\Uninstall ${PRODUCT_NAME}.lnk" "$INSTDIR\uninst.exe"

  WriteUninstaller "$INSTDIR\uninst.exe"
  WriteRegStr HKCU "${DIR_REGKEY}" "" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "$(^Name)"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" "$INSTDIR\uninst.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "${WEB_SITE}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "sandraschi"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section Uninstall
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\Uninstall ${PRODUCT_NAME}.lnk"
  RMDir "$SMPROGRAMS\${PRODUCT_NAME}"
  Delete "$INSTDIR\${APP_EXE}"
!ifdef CLI_EXE
  Delete "$INSTDIR\${CLI_EXE}"
!endif
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\HELP.md"
  Delete "$INSTDIR\CHANGELOG.md"
  Delete "$INSTDIR\uninst.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "${DIR_REGKEY}"
  SetAutoClose true
SectionEnd
