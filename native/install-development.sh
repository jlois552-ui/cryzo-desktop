#!/usr/bin/env sh
# Builds and installs a local Cryzo development build on macOS or Linux, as install-development.ps1
# does on Windows. Run it from anywhere in the checkout: sh native/install-development.sh
# Needs Rust (rustup), Bun, and on Linux the WebKitGTK build packages (see native/README.md).
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
# Cryzo Free, dictation, Publish and remote control need the gateway compiled in.
: "${CRYZO_GATEWAY_URL:=https://benevolent-rhinoceros-399.convex.site}"
export CRYZO_GATEWAY_URL

cd "$repo/packages/desktop-tauri"
bun install --frozen-lockfile >/dev/null

case "$(uname -s)" in
Darwin)
  # An .app bundle carries Info.plist, so macOS asks for the microphone, Accessibility and Screen
  # Recording in Cryzo's name.
  bunx tauri build --debug --bundles app
  app="$repo/native/target/debug/bundle/macos/Cryzo Desktop.app"
  target="/Applications/Cryzo Development.app"
  pkill -x "Cryzo Desktop" 2>/dev/null || true
  rm -rf "$target"
  cp -R "$app" "$target"
  echo "Installed $target"
  open "$target"
  ;;
Linux)
  bun run build:ui
  (cd "$repo/native" && cargo build -p cryzo-desktop --features tauri/custom-protocol)
  bin="$HOME/.local/bin"
  share="$HOME/.local/share"
  mkdir -p "$bin" "$share/applications" "$share/icons/hicolor/128x128/apps"
  pkill -x cryzo-development 2>/dev/null || true
  install -m 755 "$repo/native/target/debug/cryzo-desktop" "$bin/cryzo-development"
  install -m 644 "$repo/packages/desktop-tauri/src-tauri/icons/128x128.png" \
    "$share/icons/hicolor/128x128/apps/cryzo-development.png"
  cat >"$share/applications/cryzo-development.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Cryzo Development
Comment=Cryzo local development build
Exec=$bin/cryzo-development
Icon=cryzo-development
Categories=Development;
StartupWMClass=cryzo-desktop
EOF
  echo "Installed $bin/cryzo-development"
  nohup "$bin/cryzo-development" >/dev/null 2>&1 &
  ;;
*)
  echo "On Windows, use native/install-development.ps1." >&2
  exit 1
  ;;
esac
