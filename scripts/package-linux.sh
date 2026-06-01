#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="target_linux"
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
  aarch64|arm64) deb_arch="arm64"; artifact_arch="arm64" ;;
  *) deb_arch="amd64"; artifact_arch="x64" ;;
esac

if [[ -z "$mihomo_asset_pattern" ]]; then
  if [[ "$artifact_arch" == "arm64" ]]; then
    mihomo_asset_pattern="mihomo-linux-arm64-v*.gz"
  else
    mihomo_asset_pattern="mihomo-linux-amd64-compatible-v*.gz"
  fi
fi

version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_root/Cargo.toml" | head -n 1)"
export CARGO_TARGET_DIR="$project_root/$target_dir"

cd "$project_root"
cargo build --release

binary="$CARGO_TARGET_DIR/release/chatgpt_webview_client"
artifacts="$project_root/$target_dir/artifacts"
portable_dir="$project_root/$target_dir/portable/$package_name"
deb_root="$project_root/$target_dir/deb-root"

rm -rf "$artifacts" "$portable_dir" "$deb_root"
mkdir -p "$artifacts" "$portable_dir/resources/clash"

cp "$binary" "$portable_dir/easygpt-bin"
chmod +x "$portable_dir/easygpt-bin"
cat > "$portable_dir/EasyGPT" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
app_dir="$(cd "$(dirname "$0")" && pwd)"
export EASYGPT_DATA_DIR="${EASYGPT_DATA_DIR:-$app_dir/data}"
exec "$app_dir/easygpt-bin" "$@"
SH
chmod +x "$portable_dir/EasyGPT"

bash "$project_root/scripts/ensure-mihomo.sh" \
  --destination "$portable_dir/resources/clash/mihomo" \
  --asset-pattern "$mihomo_asset_pattern" \
  --force

cp "$project_root/README.md" "$portable_dir/README.md"
cp "$project_root/THIRD_PARTY_NOTICES.txt" "$portable_dir/THIRD_PARTY_NOTICES.txt"
tar -czf "$artifacts/EasyGPT-linux-$artifact_arch-portable.tar.gz" -C "$project_root/$target_dir/portable" "$package_name"

mkdir -p "$deb_root/DEBIAN" "$deb_root/opt/easygpt/resources/clash" "$deb_root/usr/bin" "$deb_root/usr/share/applications"
cp "$binary" "$deb_root/opt/easygpt/easygpt-bin"
chmod +x "$deb_root/opt/easygpt/easygpt-bin"
cp "$portable_dir/resources/clash/mihomo" "$deb_root/opt/easygpt/resources/clash/mihomo"
chmod +x "$deb_root/opt/easygpt/resources/clash/mihomo"
cp "$project_root/README.md" "$deb_root/opt/easygpt/README.md"
cp "$project_root/THIRD_PARTY_NOTICES.txt" "$deb_root/opt/easygpt/THIRD_PARTY_NOTICES.txt"

cat > "$deb_root/usr/bin/easygpt" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
data_base="${XDG_DATA_HOME:-$HOME/.local/share}"
export EASYGPT_DATA_DIR="${EASYGPT_DATA_DIR:-$data_base/EasyGPT/data}"
exec /opt/easygpt/easygpt-bin "$@"
SH
chmod +x "$deb_root/usr/bin/easygpt"

cat > "$deb_root/usr/share/applications/easygpt.desktop" <<'DESKTOP'
[Desktop Entry]
Name=EasyGPT
Comment=AI desktop client with app-local proxy support
Exec=easygpt
Terminal=false
Type=Application
Categories=Network;Utility;
DESKTOP

cat > "$deb_root/DEBIAN/control" <<CONTROL
Package: easygpt
Version: $version
Section: net
Priority: optional
Architecture: $deb_arch
Maintainer: EasyGPT <noreply@github.com>
Depends: libwebkit2gtk-4.1-0 | libwebkit2gtk-4.0-37, libgtk-3-0, libxdo3, libssl3 | libssl1.1
Description: EasyGPT AI desktop client
 EasyGPT is a lightweight desktop client for ChatGPT and related AI sites.
 It embeds a WebView and can run an app-local mihomo proxy runtime.
CONTROL

dpkg-deb --build "$deb_root" "$artifacts/EasyGPT-linux-$artifact_arch.deb"

echo "Linux artifacts created:"
find "$artifacts" -maxdepth 1 -type f -print
