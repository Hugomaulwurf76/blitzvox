"""
Übernahme von Daten der früheren Version "Blitztext" nach "BlitzVox".

Läuft bei jedem Start, ist aber harmlos, wenn nichts zu tun ist:
  1. Einstellungsordner (settings.json, corrections.json usw.):
     wird nur kopiert, wenn BlitzVox noch keine settings.json hat.
  2. API-Keys im Schlüsselbund (keyring):
     werden nur kopiert, wenn unter BlitzVox noch kein Key mit diesem Namen liegt.
  3. Autostart: Der alte Eintrag "Blitztext" zeigt auf ein Programm, das es nicht mehr gibt.
     Er wird entfernt und, falls Autostart eingeschaltet war, als "BlitzVox" neu angelegt.

Die alten Daten werden nicht gelöscht (Einstellungsordner und Keys bleiben als Sicherung).
Fehler werden nur in migration.log geschrieben und halten den Start nie auf.
"""

import os
import platform
import shutil
import traceback
from pathlib import Path

OLD_SERVICE_NAME = "Blitztext"
OLD_CONFIG_DIR = "blitztext"
OLD_RUN_VALUE = "Blitztext"
OLD_DESKTOP_FILE = "blitztext.desktop"

# Dateien, die nicht übernommen werden
_SKIP_FILES = {"instance.lock"}


def _config_base() -> Path:
    if os.name == "nt":
        return Path(os.environ.get("APPDATA", Path.home()))
    return Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))


def _log(new_dir: Path, text: str) -> None:
    try:
        with open(new_dir / "migration.log", "a", encoding="utf-8") as f:
            f.write(text + "\n")
    except Exception:
        pass


# ── 1. Einstellungsordner ─────────────────────────────────────────────────────

def _migrate_config(new_dir: Path) -> bool:
    """Kopiert den alten Ordner, wenn BlitzVox noch keine Einstellungen hat.
    Gibt True zurück, wenn kopiert wurde."""
    old_dir = _config_base() / OLD_CONFIG_DIR
    if not old_dir.is_dir() or (new_dir / "settings.json").exists():
        return False
    for item in old_dir.iterdir():
        if item.name in _SKIP_FILES:
            continue
        target = new_dir / item.name
        if target.exists():
            continue
        if item.is_dir():
            shutil.copytree(item, target)
        else:
            shutil.copy2(item, target)
    _log(new_dir, f"Einstellungen aus {old_dir} übernommen.")
    return True


# ── 2. API-Keys ───────────────────────────────────────────────────────────────

def _migrate_keys(new_dir: Path) -> None:
    import keyring
    from . import settings as st

    names = [st.KEY_OPENAI, st.KEY_GROQ, st.KEY_ANTHROPIC, st.KEY_ASSEMBLYAI,
             st.KEY_CUSTOM_STT, st.KEY_CUSTOM_LLM, st.KEY_OPENROUTER]
    copied = []
    for name in names:
        try:
            if keyring.get_password(st.SERVICE_NAME, name):
                continue
            old = keyring.get_password(OLD_SERVICE_NAME, name)
            if old:
                keyring.set_password(st.SERVICE_NAME, name, old)
                copied.append(name)
        except Exception:
            _log(new_dir, f"Key {name}: " + traceback.format_exc())
    if copied:
        _log(new_dir, "API-Keys übernommen: " + ", ".join(copied))


# ── 3. Autostart ──────────────────────────────────────────────────────────────

def _remove_old_autostart() -> bool:
    """Entfernt den alten Autostart-Eintrag. True, wenn einer vorhanden war."""
    system = platform.system()
    if system == "Windows":
        import winreg
        from .autostart import RUN_KEY
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0,
                                winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE) as key:
                winreg.QueryValueEx(key, OLD_RUN_VALUE)
                winreg.DeleteValue(key, OLD_RUN_VALUE)
                return True
        except OSError:
            return False
    if system == "Linux":
        base = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
        path = Path(base) / "autostart" / OLD_DESKTOP_FILE
        if path.exists():
            path.unlink()
            return True
    return False


def _migrate_autostart(new_dir: Path) -> None:
    if not _remove_old_autostart():
        return
    from .settings import load_settings
    from .autostart import set_enabled
    if load_settings().launch_at_login:
        set_enabled(True)
        _log(new_dir, "Autostart von Blitztext auf BlitzVox umgestellt.")
    else:
        _log(new_dir, "Alten Autostart-Eintrag von Blitztext entfernt.")


# ── Öffentlich ────────────────────────────────────────────────────────────────

def migrate_from_blitztext() -> None:
    """Einmal beim Start aufrufen, bevor die Einstellungen geladen werden."""
    from .settings import _config_path
    new_dir = _config_path().parent
    for step in (_migrate_config, _migrate_keys, _migrate_autostart):
        try:
            step(new_dir)
        except Exception:
            _log(new_dir, f"{step.__name__}: " + traceback.format_exc())
