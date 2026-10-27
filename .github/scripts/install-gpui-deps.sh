#!/usr/bin/env bash
set -euo pipefail

# GitHub-hosted ubuntu-24.04 has repeatedly routed apt through a slow or
# unreachable mirror. Try Canonical's archive first, then well-known mirrors,
# with a hard cap per attempt so a dead host cannot stall the job. Apt still
# verifies every package signature. UBUNTU_MIRROR overrides the list entirely.
mirrors=()
if [[ -n "${UBUNTU_MIRROR:-}" ]]; then
  mirrors+=("$UBUNTU_MIRROR")
fi
mirrors+=(
  "https://archive.ubuntu.com/ubuntu"
  "https://mirrors.mit.edu/ubuntu"
  "https://mirror.csclub.uwaterloo.ca/ubuntu"
  "https://mirror.umd.edu/ubuntu"
  "https://mirrors.edge.kernel.org/ubuntu"
)

write_mirror() {
  local mirror="$1"
  if [[ -f /etc/apt/apt-mirrors.txt ]]; then
    printf '%s\n' "$mirror" | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
  fi
  for source in /etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources; do
    if [[ -f "$source" ]]; then
      sudo sed -i -E \
        "s#^URIs:.*#URIs: ${mirror}#; s#https?://[^ ]+/ubuntu#${mirror}#g" \
        "$source"
    fi
  done
}

# Fail fast on a bad mirror instead of letting the job sit until timeout.
apt_options=(
  -o Acquire::Retries=2
  -o Acquire::http::Timeout=20
  -o Acquire::https::Timeout=20
  -o Acquire::ForceIPv4=true
)

selected_mirror=""
for mirror in "${mirrors[@]}"; do
  echo "Trying Ubuntu apt mirror: ${mirror}"
  write_mirror "$mirror"
  if sudo timeout --signal=TERM 90 apt-get "${apt_options[@]}" update; then
    selected_mirror="$mirror"
    break
  fi
  echo "Mirror failed, trying the next one: ${mirror}" >&2
done
if [[ -z "$selected_mirror" ]]; then
  echo "No Ubuntu apt mirror was reachable." >&2
  exit 1
fi
echo "Using Ubuntu apt mirror: ${selected_mirror}"

sudo timeout --signal=TERM 300 apt-get "${apt_options[@]}" install -y --no-install-recommends \
  gcc g++ clang libfontconfig-dev libwayland-dev \
  libwebkit2gtk-4.1-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libssl-dev libzstd-dev libvulkan1
