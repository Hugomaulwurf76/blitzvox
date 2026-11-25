#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    # BlitzVox 1.0.8 lag unter /opt/blitzvox und hat den Autostart
    # (~/.config/autostart/blitzvox.desktop) mit Exec="/opt/blitzvox/blitzvox"
    # angelegt. Damit dieser Eintrag nach dem Update weiter funktioniert,
    # zeigt der alte Pfad auf das neue Programm.
    mkdir -p /opt/blitzvox
    ln -sfn /usr/bin/blitzvox /opt/blitzvox/blitzvox

    # udev-Regel für Tastatur und virtuelle Tastatur sofort wirksam machen
    # (sonst erst nach dem nächsten Neustart).
    modprobe uinput 2>/dev/null || true
    if command -v udevadm >/dev/null 2>&1; then
        udevadm control --reload-rules || true
        udevadm trigger --subsystem-match=input --action=change || true
        udevadm trigger --subsystem-match=misc --sysname-match=uinput --action=change || true
    fi
fi
exit 0
