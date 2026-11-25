#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "purge" ]; then
    # Verweis aus postinst.sh löschen
    if [ -L /opt/blitzvox/blitzvox ]; then
        rm -f /opt/blitzvox/blitzvox
    fi
    rmdir /opt/blitzvox 2>/dev/null || true
    # udev-Regel ist entfernt – Rechte neu setzen
    if command -v udevadm >/dev/null 2>&1; then
        udevadm control --reload-rules || true
        udevadm trigger --subsystem-match=input --action=change || true
        udevadm trigger --subsystem-match=misc --sysname-match=uinput --action=change || true
    fi
fi
exit 0
