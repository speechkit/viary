#!/usr/bin/env bash
# Runs a headless GNOME Shell (Wayland, as GNOME 50 always is) with
# Viary's extension, then checks what Viary relies on: the extension
# answers on D-Bus, and the portals for the talk shortcut and for typing
# are there. Ends with the
# Linux tests that need a running GNOME (`cargo test -- --ignored gnome_`).
#
# Run inside its own session bus: dbus-run-session -- ci/gnome-wayland.sh
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
uuid="viary@viary.app"

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
chmod 700 "$XDG_RUNTIME_DIR"
export XDG_SESSION_TYPE=wayland
export XDG_CURRENT_DESKTOP=GNOME
# Services D-Bus starts on demand, the desktop portal among them, see
# only this environment. Without XDG_CURRENT_DESKTOP the portal falls back
# to its GTK backends, which have no GlobalShortcuts or RemoteDesktop. The
# display names are the ones GNOME Shell takes; they are not exported, so
# the shell itself does not run as a nested client.
dbus-update-activation-environment \
  XDG_RUNTIME_DIR XDG_SESSION_TYPE XDG_CURRENT_DESKTOP WAYLAND_DISPLAY=wayland-0 DISPLAY=:0

# The extension, installed as Viary's setup window installs it.
extensions="$HOME/.local/share/gnome-shell/extensions/$uuid"
mkdir -p "$extensions"
cp "$repo/gnome-extension/$uuid/"* "$extensions/"
gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.shell enabled-extensions "['$uuid']"

log="$(mktemp)"
gnome-shell --headless --virtual-monitor 1280x800 >"$log" 2>&1 &
shell=$!
trap 'kill $shell 2>/dev/null || true; echo "--- gnome-shell log"; cat "$log"' EXIT

wait_for() {
  local name=$1
  for _ in $(seq 1 60); do
    if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.NameHasOwner "$name" 2>/dev/null | grep -q true; then
      return 0
    fi
    sleep 1
  done
  echo "::error::$name never appeared on the session bus"
  return 1
}

wait_for org.gnome.Shell
gnome-shell --version
wait_for app.viary.Shell

viary() {
  gdbus call --session --dest app.viary.Shell --object-path /app/viary/Shell "$@"
}

echo "--- the extension"
viary --method org.freedesktop.DBus.Properties.Get app.viary.Shell Version
viary --method app.viary.Shell.FocusedApp
viary --method app.viary.Shell.ShowPill \
  '{"kind":"listening","token":1,"startedAt":0,"context":"CI","live":true}'
viary --method app.viary.Shell.SetLevel 0.1
viary --method app.viary.Shell.ShowPill '{"kind":"idle"}'
viary --method app.viary.Shell.Paste

echo "--- the portals"
portal() {
  gdbus call --session --dest org.freedesktop.portal.Desktop \
    --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.DBus.Properties.Get "org.freedesktop.portal.$1" version
}
# Hold-to-talk needs GlobalShortcuts; typing without the extension,
# RemoteDesktop.
portal GlobalShortcuts
portal RemoteDesktop

echo "--- Viary's tests against GNOME"
cd "$repo/src-tauri"
cargo test --lib -- --ignored gnome_
