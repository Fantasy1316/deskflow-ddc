; Never leave an installed privileged core behind when replacing/removing the UI.
!macro DDC_CLEAN_SERVICE EXECUTABLE
  ClearErrors
  ExecWait '"${EXECUTABLE}" --cleanup-windows-service' $0
  ${If} ${Errors}
  ${OrIf} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "Deskflow DDC 后台服务尚未清理。请停止共享，并允许管理员授权后重试。"
    Abort
  ${EndIf}
!macroend
!macro NSIS_HOOK_PREINSTALL
  ; Use the NEW executable: older versions do not understand the cleanup flag
  ; and would open their GUI, leaving an upgrading installer waiting forever.
  InitPluginsDir
  File "/oname=$PLUGINSDIR\ddc-service-cleanup.exe" "${MAINBINARYSRCPATH}"
  !insertmacro DDC_CLEAN_SERVICE "$PLUGINSDIR\ddc-service-cleanup.exe"
  Delete "$PLUGINSDIR\ddc-service-cleanup.exe"
!macroend
!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro DDC_CLEAN_SERVICE "$INSTDIR\${MAINBINARYNAME}.exe"
  ${EndIf}
!macroend
