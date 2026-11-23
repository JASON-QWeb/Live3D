#!/bin/bash
set -euo pipefail
project="$(cd "$(dirname "$0")/.." && pwd)"
zip=false
if [[ "${1:-}" == "--zip" ]]; then zip=true; shift; fi
if [[ $# -gt 1 || "${1:-}" == --* ]]; then
  echo "用法: $0 [--zip] [输出目录]" >&2
  exit 2
fi
mkdir -p "${1:-$project/dist}"
dest="$(cd "${1:-$project/dist}" && pwd)"
cd "$project"
export MACOSX_DEPLOYMENT_TARGET=14.0
stage="$(mktemp -d "$dest/.vorela-build.XXXXXX")"
app="$stage/Vorela.app"
output="$dest/Vorela.app"
trap 'if [[ ! -e "$stage/previous.app" ]]; then rm -rf "$stage"; fi' EXIT
identity="${VORELA_SIGN_IDENTITY:--}"
sign=(--force --sign "$identity")
if [[ "$identity" != "-" ]]; then sign+=(--options runtime); fi
executable="$(cargo build --locked --release --bin vorela --message-format=json-render-diagnostics | python3 -c '
import json, sys
executable = None
for message in map(json.loads, sys.stdin):
    if (message["reason"] == "compiler-artifact" and message["target"]["name"] == "vorela"
            and message["executable"]):
        executable = message["executable"]
if executable is None:
    sys.exit("Cargo 未生成 vorela 可执行文件")
print(executable)
')"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 644 packaging/Info.plist "$app/Contents/Info.plist"
version="$("$executable" --version)"
plutil -insert CFBundleShortVersionString -string "${version#Vorela }" "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
install -m 755 "$executable" "$app/Contents/MacOS/vorela"
install -m 644 packaging/Vorela.icns "$app/Contents/Resources/Vorela.icns"
install -m 644 LICENSE "$app/Contents/Resources/LICENSE"
install -m 644 NOTICE.md "$app/Contents/Resources/NOTICE.md"
install -m 644 README.md "$app/Contents/Resources/README.md"
install -m 644 scripts/inspect-recording.py "$app/Contents/Resources/inspect-recording.py"
for asset in avatars/cn/xiaorou_knowledge_base.txt avatars/en/knowledge_base.txt avatars/{cn,en}/song_titles.txt; do
  if [[ -f "$asset" ]]; then
    mkdir -p "$app/Contents/Resources/$(dirname "$asset")"
    install -m 644 "$asset" "$app/Contents/Resources/$asset"
  fi
done
python3 scripts/notices.py "$app/Contents/Resources/ThirdParty.txt"
codesign "${sign[@]}" --identifier studio.vorela.control "$app"
codesign --verify --deep --strict "$app"
if "$zip"; then ditto -c -k --norsrc --keepParent "$app" "$stage/Vorela.zip"; fi
if [[ -e "$output" ]]; then
  existing_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$output/Contents/Info.plist")"
  [[ "$existing_id" == "studio.vorela.control" ]] || { echo "目标位置有其他应用" >&2; exit 1; }
  mv "$output" "$stage/previous.app"
fi
if ! mv "$app" "$output"; then
  if [[ -e "$stage/previous.app" ]]; then mv "$stage/previous.app" "$output"; fi
  exit 1
fi
rm -rf "$stage/previous.app"
echo "$output"
if "$zip"; then
  mv -f "$stage/Vorela.zip" "$dest/Vorela.zip"
  echo "$dest/Vorela.zip"
fi
