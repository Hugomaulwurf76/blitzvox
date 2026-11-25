@echo off
rem BlitzVox im Entwicklungsmodus starten (cargo tauri dev).
rem Der Build-Ordner muss der kurze Pfad D:\bv sein, sonst scheitert
rem whisper-rs mit Vulkan unter Windows an zu langen Pfaden.
setlocal
if not exist "D:\bv\" (
  echo Fehler: Die Verknuepfung D:\bv fehlt. Sie muss auf D:\Programme\Blitzvox\target zeigen.
  echo Anlegen mit:  mklink /J D:\bv D:\Programme\Blitzvox\target
  exit /b 1
)
set "CARGO_TARGET_DIR=D:\bv"
cd /d "%~dp0src-tauri"
cargo tauri dev %*
exit /b %ERRORLEVEL%
