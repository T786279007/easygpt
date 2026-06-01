#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
destination=""
asset_pattern=""
force=0

case "$(uname -s)" in
  Darwin)
    case "$(uname -m)" in
      arm64) asset_pattern="mihomo-darwin-arm64-v*.gz" ;;
      *) asset_pattern="mihomo-darwin-amd64-compatible-v*.gz" ;;
    esac
    ;;
  Linux)
    case "$(uname -m)" in
      aarch64|arm64) asset_pattern="mihomo-linux-arm64-v*.gz" ;;
      *) asset_pattern="mihomo-linux-amd64-compatible-v*.gz" ;;
    esac
    ;;
  *)
    echo "Unsupported system for ensure-mihomo.sh: $(uname -s)" >&2
    exit 1
    ;;
esac

while [[ $# -gt 0 ]]; do
  case "$1" in
    --destination)
      destination="${2:?missing --destination value}"
      shift 2
      ;;
    --asset-pattern)
      asset_pattern="${2:?missing --asset-pattern value}"
      shift 2
      ;;
    --force)
      force=1
      shift
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ -z "$destination" ]]; then
  destination="$project_root/resources/clash/mihomo"
fi

if [[ -f "$destination" && "$force" -ne 1 ]]; then
  echo "mihomo already exists:"
  echo "$destination"
  exit 0
fi

mkdir -p "$(dirname "$destination")"
work_dir="$project_root/target/mihomo-download-unix"
rm -rf "$work_dir"
mkdir -p "$work_dir"

python3 - "$asset_pattern" "$work_dir" <<'PY'
import fnmatch
import json
import pathlib
import sys
import urllib.request

pattern = sys.argv[1]
work_dir = pathlib.Path(sys.argv[2])
request = urllib.request.Request(
    "https://api.github.com/repos/MetaCubeX/mihomo/releases/latest",
    headers={"User-Agent": "easygpt-packager"},
)
with urllib.request.urlopen(request, timeout=60) as response:
    release = json.load(response)

assets = release.get("assets", [])
asset = next((item for item in assets if fnmatch.fnmatch(item.get("name", ""), pattern)), None)
if asset is None:
    names = ", ".join(item.get("name", "") for item in assets)
    raise SystemExit(f"Could not find mihomo asset matching {pattern!r}. Available assets: {names}")

archive_path = work_dir / asset["name"]
print("Downloading mihomo:")
print(asset["browser_download_url"])
urllib.request.urlretrieve(asset["browser_download_url"], archive_path)
print(archive_path)
PY

archive_path="$(find "$work_dir" -maxdepth 1 -type f | head -n 1)"
case "$archive_path" in
  *.gz)
    gzip -dc "$archive_path" > "$destination"
    ;;
  *)
    echo "Unsupported mihomo archive type: $archive_path" >&2
    exit 1
    ;;
esac

chmod +x "$destination"
echo "mihomo prepared:"
echo "$destination"
