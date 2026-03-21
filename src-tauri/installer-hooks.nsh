!macro NSIS_HOOK_PREINSTALL
  IfFileExists "$INSTDIR\WinDivert64.sys" 0 hook_done

  ClearErrors
  Delete "$INSTDIR\WinDivert.dll"
  Delete "$INSTDIR\WinDivert64.sys"

  IfErrors hook_busy hook_done

  hook_busy:
    MessageBox MB_OK|MB_ICONSTOP "WinDivert64.sys is in use and could not be updated.$\r$\n$\r$\nClose resonance-chat and run the installer again.$\r$\nIf it still fails, reboot Windows and retry, or use the portable zip release."
    Abort

  hook_done:
!macroend
