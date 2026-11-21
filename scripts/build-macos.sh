#!/bin/bash
set -euo pipefail
project="$(cd "$(dirname "$0")/.." && pwd)"
cd "$project"
target_dir="${CARGO_TARGET_DIR:-$project/target}"
dest="${1:-$project/dist}"
cargo build --locked --release --target-dir "$target_dir"
mkdir -p "$dest"
stage="$(mktemp -d "$dest/.avatar3d-build.XXXXXX")"
app="$stage/Avatar3D Live.app"
output="$dest/Avatar3D Live.app"
trap 'if [[ ! -e "$stage/previous.app" ]]; then rm -rf "$stage"; fi' EXIT
identity="${AVATAR3D_SIGN_IDENTITY:--}"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Helpers" "$app/Contents/Resources"
install -m 644 packaging/Info.plist "$app/Contents/Info.plist"
install -m 755 "$target_dir/release/avatar3d-live" "$app/Contents/Helpers/avatar3d-live"
xcrun clang -fobjc-arc -Os -mmacosx-version-min=14.0 -framework Foundation \
  packaging/Launcher.m -o "$app/Contents/MacOS/Avatar3DLauncher"
codesign --force --sign "$identity" --identifier studio.avatar3d.core "$app/Contents/Helpers/avatar3d-live"
install -m 644 packaging/Avatar3D.icns "$app/Contents/Resources/Avatar3D.icns"
install -m 644 LICENSE "$app/Contents/Resources/LICENSE"
install -m 644 NOTICE.md "$app/Contents/Resources/NOTICE.md"
install -m 644 README.md "$app/Contents/Resources/README.md"
install -m 644 scripts/inspect-recording.py "$app/Contents/Resources/inspect-recording.py"
python3 scripts/notices.py "$app/Contents/Resources/ThirdParty.txt"
codesign --force --sign "$identity" --identifier studio.avatar3d.control "$app"
codesign --verify --deep --strict "$app"
if [[ -e "$output" ]]; then
  existing_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$output/Contents/Info.plist")"
  [[ "$existing_id" == "studio.avatar3d.control" ]] || { echo "目标位置有其他应用" >&2; exit 1; }
  mv "$output" "$stage/previous.app"
fi
if ! mv "$app" "$output"; then
  if [[ -e "$stage/previous.app" ]]; then mv "$stage/previous.app" "$output"; fi
  exit 1
fi
rm -rf "$stage/previous.app"
echo "$output"
