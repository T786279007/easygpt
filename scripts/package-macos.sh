#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="target_macos"
package_name="EasyGPT"
arch="$(uname -m)"
mihomo_asset_pattern=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-dir)
      target_dir="${2:?missing --target-dir value}"
      shift 2
      ;;
    --arch)
      arch="${2:?missing --arch value}"
      shift 2
      ;;
    --mihomo-asset-pattern)
      mihomo_asset_pattern="${2:?missing --mihomo-asset-pattern value}"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

case "$arch" in
  arm64|aarch64) arch="arm64" ;;
  x86_64|amd64) arch="x64" ;;
esac

if [[ -z "$mihomo_asset_pattern" ]]; then
  if [[ "$arch" == "arm64" ]]; then
    mihomo_asset_pattern="mihomo-darwin-arm64-v*.gz"
  else
    mihomo_asset_pattern="mihomo-darwin-amd64-compatible-v*.gz"
  fi
fi

version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_root/Cargo.toml" | head -n 1)"
export CARGO_TARGET_DIR="$project_root/$target_dir"

cd "$project_root"
cargo build --release

binary="$CARGO_TARGET_DIR/release/chatgpt_webview_client"
package_root="$project_root/$target_dir/package"
app_dir="$package_root/$package_name.app"
artifacts="$project_root/$target_dir/artifacts"

rm -rf "$package_root" "$artifacts"
mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources/clash" "$artifacts"

cp "$binary" "$app_dir/Contents/MacOS/easygpt-bin"
chmod +x "$app_dir/Contents/MacOS/easygpt-bin"

cat > "$app_dir/Contents/MacOS/$package_name" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
contents_dir="$(cd "$(dirname "$0")/.." && pwd)"
export EASYGPT_DATA_DIR="${EASYGPT_DATA_DIR:-$HOME/Library/Application Support/EasyGPT/data}"
exec "$contents_dir/MacOS/easygpt-bin" "$@"
SH
chmod +x "$app_dir/Contents/MacOS/$package_name"

cat > "$app_dir/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>$package_name</string>
  <key>CFBundleDisplayName</key>
  <string>$package_name</string>
  <key>CFBundleExecutable</key>
  <string>$package_name</string>
  <key>CFBundleIdentifier</key>
  <string>com.easygpt.client</string>
  <key>CFBundleVersion</key>
  <string>$version</string>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

bash "$project_root/scripts/ensure-mihomo.sh" \
  --destination "$app_dir/Contents/Resources/clash/mihomo" \
  --asset-pattern "$mihomo_asset_pattern" \
  --force

cp "$project_root/README.md" "$app_dir/Contents/Resources/README.md"
cp "$project_root/THIRD_PARTY_NOTICES.txt" "$app_dir/Contents/Resources/THIRD_PARTY_NOTICES.txt"

tar -czf "$artifacts/EasyGPT-macos-$arch-app.tar.gz" -C "$package_root" "$package_name.app"

dmg_root="$project_root/$target_dir/dmg"
rm -rf "$dmg_root"
mkdir -p "$dmg_root"
cp -R "$app_dir" "$dmg_root/"
ln -s /Applications "$dmg_root/Applications"
hdiutil create -volname "$package_name" -srcfolder "$dmg_root" -ov -format UDZO "$artifacts/EasyGPT-macos-$arch.dmg"

echo "macOS artifacts created:"
find "$artifacts" -maxdepth 1 -type f -print
