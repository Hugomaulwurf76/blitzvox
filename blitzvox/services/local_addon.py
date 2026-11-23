"""
Optionale Erweiterung „Lokale Spracherkennung“ (faster-whisper).

faster-whisper ist nicht fest in BlitzVox eingebaut, weil es den Download
stark vergrößert. Es liegt als Erweiterung im Ordner „lokal“:
  - Windows (Installer):  neben BlitzVox.exe, z. B. C:\\Program Files\\BlitzVox\\lokal
  - Linux (.deb):         /opt/blitzvox/lokal  (Paket „blitzvox-lokal“)
  - Start aus Quellcode:  Projektordner/lokal, oder faster-whisper per pip
                          im verwendeten Python installiert

enable() muss beim Programmstart einmal aufgerufen werden.
"""

import os
import platform
import sys
from pathlib import Path
from typing import Optional

ADDON_DIR_NAME = "lokal"

_available: Optional[bool] = None
_error: str = ""


def is_frozen() -> bool:
    """True bei EXE bzw. .deb (mit PyInstaller gebaut), False beim Start aus dem Quellcode."""
    return bool(getattr(sys, "frozen", False))


def addon_dir() -> Path:
    if is_frozen():
        return Path(sys.executable).resolve().parent / ADDON_DIR_NAME
    return Path(__file__).resolve().parents[2] / ADDON_DIR_NAME


def enable() -> None:
    """Erweiterungsordner einbinden, falls vorhanden. Wird hinten an den
    Suchpfad gehängt, damit BlitzVox' eigene Pakete (z. B. numpy) Vorrang haben."""
    d = addon_dir()
    if not d.is_dir():
        return
    path = str(d)
    if path not in sys.path:
        sys.path.append(path)
    if os.name == "nt":
        try:
            os.add_dll_directory(path)
        except (OSError, AttributeError):
            pass


def is_available() -> bool:
    """True, wenn faster-whisper wirklich geladen werden kann (Ergebnis wird gemerkt)."""
    global _available, _error
    if _available is None:
        try:
            import faster_whisper  # noqa: F401
            _available = True
        except Exception as exc:
            _available = False
            _error = f"{type(exc).__name__}: {exc}"
    return _available


def load_error() -> str:
    """Fehlermeldung, falls die Erweiterung vorhanden, aber nicht ladbar ist."""
    return _error


def cuda_device_count() -> int:
    """Anzahl der NVIDIA-GPUs, die CTranslate2 sieht (0 ohne GPU oder ohne Treiber)."""
    try:
        import ctranslate2
        return int(ctranslate2.get_cuda_device_count())
    except Exception:
        return 0


def install_mode() -> str:
    """'windows' (Installer), 'linux' (.deb) oder 'source' (Quellcode).

    Zum Testen aus dem Quellcode lässt sich die Installationsart vorgeben,
    damit man die Texte sieht, die Endanwender sehen:
      - Datei „testmodus.txt“ im Projektordner (neben main.py) mit dem Inhalt
        windows  bzw.  linux   (gilt nur beim Start aus dem Quellcode), oder
      - Umgebungsvariable BLITZVOX_INSTALL_MODE=windows bzw. linux
    """
    valid = ("windows", "linux", "source")
    forced = os.environ.get("BLITZVOX_INSTALL_MODE", "").strip().lower()
    if forced in valid:
        return forced
    if not is_frozen():
        try:
            f = Path(__file__).resolve().parents[2] / "testmodus.txt"
            forced = f.read_text(encoding="utf-8-sig").strip().lower()
            if forced in valid:
                return forced
        except OSError:
            pass
        return "source"
    return "windows" if platform.system() == "Windows" else "linux"


def missing_message() -> str:
    """Kurze Fehlermeldung fürs Workflow-Fenster, wenn „Lokal“ gewählt ist,
    die Erweiterung aber fehlt: sagt direkt, was je nach Installation zu tun ist."""
    from .. import __version__
    mode = install_mode()
    if addon_dir().is_dir() and load_error():
        return ("Die lokale Spracherkennung ist vorhanden, lässt sich aber nicht laden:\n"
                f"{load_error()}")
    msg = "Die lokale Spracherkennung ist nicht installiert.\n"
    if mode == "windows":
        msg += ("BlitzVox beenden, den Installer erneut ausführen und bei "
                "„Komponenten auswählen“ den Haken bei „Lokale Spracherkennung“ setzen.")
    elif mode == "linux":
        msg += ("Zusatzpaket von den GitHub-Releases installieren:\n"
                f"sudo apt install ./blitzvox-lokal_{__version__}_amd64.deb\n"
                "Danach BlitzVox neu starten.")
    else:
        msg += ("In das Python, mit dem BlitzVox läuft, installieren:\n"
                "pip install faster-whisper\n"
                "Danach BlitzVox neu starten.")
    msg += "\nOder in den Einstellungen einen Online-Dienst wählen."
    return msg
