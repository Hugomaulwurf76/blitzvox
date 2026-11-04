"""
BlitzVox Cross-Platform
Speech-to-text app for Windows 11 and Linux Debian
MIT License — Fork of the concept from cmagnussen/blitztext-app (macOS)
"""

import sys
import os
import platform

# Ensure the package is importable when running from root
sys.path.insert(0, os.path.dirname(__file__))

import traceback

from PyQt6.QtWidgets import QApplication, QMessageBox
from PyQt6.QtCore import Qt, QLockFile
from blitzvox import __version__
from blitzvox.services.settings import _config_path
from blitzvox.ui.tray import BlitzVoxTrayApp
from blitzvox.ui.theme import apply_theme
from blitzvox.services.migration import migrate_from_blitztext


def _show_unhandled(exc_type, exc, tb) -> None:
    """Zeigt unbehandelte Fehler als Fenster und schreibt sie in error.log,
    statt dass das Programm ohne Meldung beendet wird."""
    text = "".join(traceback.format_exception(exc_type, exc, tb))
    try:
        with open(_config_path().parent / "error.log", "a", encoding="utf-8") as f:
            f.write(text + "\n")
    except Exception:
        pass
    try:
        QMessageBox.critical(None, "BlitzVox – Fehler", text)
    except Exception:
        pass


def _set_windows_app_id() -> None:
    """Setzt eine eigene AppUserModelID, damit Windows die App als BlitzVox erkennt
    und nicht als python.exe / pythonw.exe in der Taskleiste anzeigt."""
    if platform.system() != "Windows":
        return
    try:
        import ctypes
        myappid = "BlitzVox.CrossPlatform.1.0"
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID(myappid)
    except Exception:
        pass


def _acquire_single_instance():
    """Sperrt eine Datei im Konfigurationsordner. Gibt None zurueck, wenn schon eine Instanz laeuft.
    QLockFile funktioniert unter Windows und Linux gleich und erkennt abgestuerzte Instanzen selbst."""
    lock_dir = _config_path().parent
    lock_dir.mkdir(parents=True, exist_ok=True)
    lock = QLockFile(str(lock_dir / "instance.lock"))
    if lock.tryLock(0):
        return lock
    return None


def main():
    _set_windows_app_id()

    # High DPI support
    QApplication.setHighDpiScaleFactorRoundingPolicy(
        Qt.HighDpiScaleFactorRoundingPolicy.PassThrough
    )

    sys.excepthook = _show_unhandled

    app = QApplication(sys.argv)
    apply_theme(app)

    lock = _acquire_single_instance()
    if lock is None:
        QMessageBox.information(
            None, "BlitzVox",
            "BlitzVox läuft bereits. Das Symbol findest du in der Taskleiste (Tray).",
        )
        sys.exit(0)
    app.single_instance_lock = lock  # Referenz halten, sonst wird die Sperre freigegeben

    # Einstellungen, API-Keys und Autostart der frueheren Version "Blitztext" uebernehmen
    migrate_from_blitztext()

    app.setApplicationName("BlitzVox")
    app.setApplicationDisplayName("BlitzVox")
    app.setApplicationVersion(__version__)
    app.setOrganizationName("BlitzVox")
    app.setDesktopFileName("blitzvox")
    # Keep running even if all windows are closed (tray app)
    app.setQuitOnLastWindowClosed(False)

    tray = BlitzVoxTrayApp(app)
    tray.show()

    sys.exit(app.exec())


if __name__ == "__main__":
    main()
