!macro NSIS_HOOK_PREINSTALL
  IfFileExists "$INSTDIR\WinDivert64.sys" 0 hook_done

  ClearErrors
  Delete "$INSTDIR\WinDivert.dll"
  Delete "$INSTDIR\WinDivert64.sys"

  IfErrors hook_busy hook_done

  hook_busy:
    MessageBox MB_OK|MB_ICONSTOP "WinDivert64.sys を更新できませんでした。現在、使用中の可能性があります。$\r$\n$\r$\nResonance Chat を終了してから、もう一度インストーラーを実行してください。$\r$\nそれでも失敗する場合は、Windows を再起動してから再度お試しください。必要に応じてポータブル版 ZIP もご利用いただけます。"
    Abort

  hook_done:
!macroend
