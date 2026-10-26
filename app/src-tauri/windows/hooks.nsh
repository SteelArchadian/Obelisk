; Obelisk installer hooks (bundle.windows.nsis.installerHooks).

; A first install asks where Obelisk keeps its own data. Updates (quiet or passive), installs over an existing
; Obelisk and PCs that already have Obelisk's data skip it: the answer stands, and Settings can change it. The
; choice goes where the app reads it (see src/datadir.rs).
!macro NSIS_HOOK_POSTINSTALL
  IfSilent obelisk_data_done
  StrCmp $PassiveMode 1 obelisk_data_done
  StrCmp $PrevInstDir "" 0 obelisk_data_done
  IfFileExists "$LOCALAPPDATA\dev.obelisk.app\*.*" obelisk_data_done
  MessageBox MB_YESNO|MB_ICONQUESTION "Obelisk keeps its settings, caches and sharing key in$\r$\n$LOCALAPPDATA\dev.obelisk.app$\r$\n$\r$\nChoose a different folder? You can change this later in Settings." IDNO obelisk_data_done
  nsDialogs::SelectFolderDialog "Where should Obelisk keep its data? (an empty or new folder)" "$LOCALAPPDATA"
  Pop $0
  StrCmp $0 "error" obelisk_data_done
  StrCmp $0 "" obelisk_data_done
  CreateDirectory "$LOCALAPPDATA\dev.obelisk.app"
  FileOpen $1 "$LOCALAPPDATA\dev.obelisk.app\data-location.txt" w
  FileWrite $1 "$0$\r$\nmove-from=$LOCALAPPDATA\dev.obelisk.app$\r$\n"
  FileClose $1
  obelisk_data_done:
!macroend
