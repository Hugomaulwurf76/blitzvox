#!/usr/bin/env bash
# BlitzVox unter Linux (Debian/Ubuntu/Linux Mint) als .deb bauen.
# Aufruf im Projektordner:  bash build-linux.sh
#
# Einmalig vorher (Linux Mint 22.x / Ubuntu 24.04):
#   sudo apt update
#   sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
#     libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
#     pkg-config cmake clang libclang-dev libvulkan-dev glslc libasound2-dev libdbus-1-dev
#   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
#   source "$HOME/.cargo/env"
#   cargo install tauri-cli --version 2.12.6 --locked

set -euo pipefail
cd "$(dirname "$0")"

fehlt=0
for cmd in cargo cmake clang glslc pkg-config; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Fehlt: $cmd (siehe Anleitung oben in dieser Datei)"
    fehlt=1
  fi
done
if ! cargo tauri --version >/dev/null 2>&1; then
  echo "Fehlt: tauri-cli (cargo install tauri-cli --version 2.12.6 --locked)"
  fehlt=1
fi
[ "$fehlt" = 0 ] || exit 1

cd src-tauri
cargo tauri build --bundles deb "$@"

echo
echo "Fertig. Paket:"
ls -1 "$(pwd)"/target/release/bundle/deb/*.deb
echo "Installieren mit:  sudo apt install ./<Dateiname>.deb"
