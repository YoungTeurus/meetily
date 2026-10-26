; ONNX Runtime 1.22.0 directly imports DXCoreCreateAdapterFactory from dxcore.dll.
; The first stable Windows release providing it is Windows 10 2004, build 19041.
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R9 HKLM "SOFTWARE\Microsoft\Windows NT\CurrentVersion" "CurrentBuildNumber"
  IntCmp $R9 19041 meetily_build_supported meetily_build_unsupported meetily_build_supported
  meetily_build_unsupported:
    MessageBox MB_OK|MB_ICONSTOP "Meetily Calls Preview requires Windows 10 version 2004 (build 19041) or newer, or Windows 11. Update Windows before installing."
    SetErrorLevel 1
    Abort
  meetily_build_supported:
!macroend
