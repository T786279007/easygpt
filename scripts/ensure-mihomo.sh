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
mkdir -p "$work_dir"

python3 - "$asset_pattern" "$work_dir" <<'PY'
import fnmatch
import json
import os
import pathlib
import shutil
import sys
import time
import urllib.error
import urllib.request

pattern = sys.argv[1]
work_dir = pathlib.Path(sys.argv[2])
token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")

headers = {
    "Accept": "application/vnd.github+json",
    "User-Agent": "easygpt-packager",
    "X-GitHub-Api-Version": "2022-11-28",
}
if token:
    headers["Authorization"] = f"Bearer {token}"


def open_with_retries(request, *, timeout=60, attempts=4):
    last_error = None
    for attempt in range(1, attempts + 1):
        try:
            return urllib.request.urlopen(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            last_error = error
            if attempt == attempts or error.code not in {403, 429, 500, 502, 503, 504}:
                raise
        except urllib.error.URLError as error:
            last_error = error
            if attempt == attempts:
                raise
        sleep_seconds = min(30, 2 ** attempt)
        print(f"Download attempt {attempt} failed: {last_error}; retrying in {sleep_seconds}s", file=sys.stderr)
        time.sleep(sleep_seconds)
    raise last_error


request = urllib.request.Request(
    "https://api.github.com/repos/MetaCubeX/mihomo/releases/latest",
    headers=headers,
)
with open_with_retries(request) as response:
    release = json.load(response)

assets = release.get("assets", [])
asset = next((item for item in assets if fnmatch.fnmatch(item.get("name", ""), pattern)), None)
if asset is None:
    names = ", ".join(item.get("name", "") for item in assets)
    raise SystemExit(f"Could not find mihomo asset matching {pattern!r}. Available assets: {names}")

archive_path = work_dir / asset["name"]
if archive_path.exists() and archive_path.stat().st_size > 0:
    print("Reusing downloaded mihomo:")
    print(archive_path)
else:
    print("Downloading mihomo:")
    print(asset["browser_download_url"])
    asset_request = urllib.request.Request(
        asset["browser_download_url"],
        headers={"User-Agent": headers["User-Agent"], **({"Authorization": headers["Authorization"]} if "Authorization" in headers else {})},
    )
    with open_with_retries(asset_request, timeout=120) as response, archive_path.open("wb") as output:
        shutil.copyfileobj(response, output)
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
