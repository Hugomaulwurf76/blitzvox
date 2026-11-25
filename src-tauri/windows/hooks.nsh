; Zusatzschritte fuer den BlitzVox-Installer (Tauri/NSIS, installerHooks).
; Datei ist UTF-8 (Umlaute in den angezeigten Texten).
;
; Vor der Installation:
;  1. BlitzVox 1.0.8 (Inno Setup) unsichtbar entfernen. Einstellungen
;     (%APPDATA%\blitzvox) und API-Keys bleiben dabei erhalten, die alte
;     Deinstallation fasst sie nicht an.
;  2. Die Vulkan-Laufzeit (offizielles Setup von LunarG) installieren, wenn
;     im System keine vulkan-1.dll vorhanden ist. Ohne sie startet BlitzVox nicht.

!define BV_OLD_UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\{B7E1F3A2-5C4D-4E8F-9A1B-6D2C3E4F5A60}_is1"

!macro NSIS_HOOK_PREINSTALL
  ; --- 1. BlitzVox 1.0.8 entfernen -------------------------------------------
  ; 1.0.8 wurde als 64-Bit-Programm installiert, ihr Eintrag liegt in der 64-Bit-Registry.
  SetRegView 64
  ReadRegStr $R5 HKLM "${BV_OLD_UNINSTKEY}" "UninstallString"
  ReadRegStr $R6 HKLM "${BV_OLD_UNINSTKEY}" "InstallLocation"
  SetRegView default
  ; InstallLocation endet mit "\" – abschneiden, damit die Pfade unten eindeutig sind.
  StrCpy $R4 $R6 "" -1
  ${If} $R4 == "\"
    StrCpy $R6 $R6 -1
  ${EndIf}

  ${If} $R5 != ""
    ; Laeuft 1.0.8 noch, wird wie bei 2.0 gefragt, ob es beendet werden darf.
    ${If} ${FileExists} "$R6\BlitzVox.exe"
      !insertmacro CheckIfAppIsRunning "$R6\BlitzVox.exe" "BlitzVox 1.0.8"
    ${EndIf}

    DetailPrint "Entferne BlitzVox 1.0.8 …"
    ExecWait '$R5 /VERYSILENT /SUPPRESSMSGBOXES /NORESTART' $R7

    ; Die Inno-Deinstallation arbeitet in einem zweiten Prozess weiter.
    ; Warten, bis ihr Eintrag und unins000.exe verschwunden sind (höchstens 60 s).
    StrCpy $R8 0
    ${Do}
      SetRegView 64
      ReadRegStr $R9 HKLM "${BV_OLD_UNINSTKEY}" "UninstallString"
      SetRegView default
      ${If} $R9 == ""
      ${AndIfNot} ${FileExists} "$R6\unins000.exe"
        ${Break}
      ${EndIf}
      Sleep 500
      IntOp $R8 $R8 + 1
    ${LoopUntil} $R8 >= 120

    ${If} $R9 != ""
      MessageBox MB_ICONSTOP "BlitzVox 1.0.8 konnte nicht entfernt werden.$\n$\nBitte deinstalliere BlitzVox 1.0.8 unter Einstellungen → Apps und starte dieses Setup danach erneut." /SD IDOK
      Abort
    ${EndIf}
    Sleep 1000
    DetailPrint "BlitzVox 1.0.8 wurde entfernt."
  ${EndIf}

  ; --- 2. Vulkan-Laufzeit ----------------------------------------------------
  ${DisableX64FSRedirection}
  ${If} ${FileExists} "$SYSDIR\vulkan-1.dll"
    ${EnableX64FSRedirection}
  ${Else}
    ${EnableX64FSRedirection}
    DetailPrint "Installiere die Vulkan-Laufzeit …"
    InitPluginsDir
    File "/oname=$PLUGINSDIR\VulkanRT.exe" "$%VULKAN_SDK%\Helpers\VulkanRT.exe"
    ExecWait '"$PLUGINSDIR\VulkanRT.exe" /S' $R7
    Delete "$PLUGINSDIR\VulkanRT.exe"
    ${If} $R7 <> 0
      MessageBox MB_ICONEXCLAMATION "Die Vulkan-Laufzeit konnte nicht installiert werden (Fehlercode $R7).$\n$\nBlitzVox braucht sie zum Starten. Bitte aktualisiere den Grafiktreiber oder installiere die Vulkan-Laufzeit von vulkan.lunarg.com." /SD IDOK
    ${Else}
      DetailPrint "Vulkan-Laufzeit installiert."
    ${EndIf}
  ${EndIf}
!macroend
