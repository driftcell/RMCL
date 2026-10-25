#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

if ! command -v sips >/dev/null || ! command -v iconutil >/dev/null; then
  echo "icon generation requires macOS sips and iconutil" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p png

render() {
  local svg="$1" size="$2" out="$3"
  sips -s format png -z "$size" "$size" "$svg" --out "$out" >/dev/null
}

for size in 16 24 32 48 64; do
  render rmcl-small.svg "$size" "png/rmcl-$size.png"
done
for size in 128 256 512 1024; do
  render rmcl.svg "$size" "png/rmcl-$size.png"
done

iconset="$tmp/rmcl.iconset"
mkdir -p "$iconset"
cp png/rmcl-16.png   "$iconset/icon_16x16.png"
cp png/rmcl-32.png   "$iconset/icon_16x16@2x.png"
cp png/rmcl-32.png   "$iconset/icon_32x32.png"
cp png/rmcl-64.png   "$iconset/icon_32x32@2x.png"
cp png/rmcl-128.png  "$iconset/icon_128x128.png"
cp png/rmcl-256.png  "$iconset/icon_128x128@2x.png"
cp png/rmcl-256.png  "$iconset/icon_256x256.png"
cp png/rmcl-512.png  "$iconset/icon_256x256@2x.png"
cp png/rmcl-512.png  "$iconset/icon_512x512.png"
cp png/rmcl-1024.png "$iconset/icon_512x512@2x.png"
iconutil -c icns "$iconset" -o rmcl.icns

python3 - <<'PY'
import struct
from pathlib import Path

root = Path(".")
sizes = [16, 24, 32, 48, 64, 128, 256]
images = [(size, (root / "png" / f"rmcl-{size}.png").read_bytes()) for size in sizes]
header = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries = bytearray()
payload = bytearray()
for size, data in images:
    dim = 0 if size >= 256 else size
    entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
    payload += data
(root / "rmcl.ico").write_bytes(header + entries + payload)
PY
