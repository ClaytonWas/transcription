#!/bin/bash
# Wrapper to run Tauri dev with clean environment (avoiding Snap pollution)

cd "$(dirname "$0")"

# Run with clean environment, preserving only essential variables
exec env -i \
  HOME="$HOME" \
  USER="$USER" \
  PATH="/usr/local/bin:/usr/bin:/bin:$HOME/.cargo/bin:$HOME/.local/bin:$HOME/.nvm/versions/node/$(ls $HOME/.nvm/versions/node 2>/dev/null | tail -1)/bin:/usr/local/go/bin" \
  DISPLAY="${DISPLAY:-:0}" \
  WAYLAND_DISPLAY="$WAYLAND_DISPLAY" \
  XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
  XDG_SESSION_TYPE="$XDG_SESSION_TYPE" \
  DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  npm run tauri dev
