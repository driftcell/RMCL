#!/usr/bin/env bash
set -euo pipefail

# GitHub-hosted ubuntu-24.04 can route apt through a slow or unreachable
# mirror. Prefer Canonical's archive and fall back to the other known hosts
# when one is down. UBUNTU_MIRROR overrides the probe entirely.
default_mirrors=(
  "https://archive.ubuntu.com/ubuntu"
  "https://azure.archive.ubuntu.com/ubuntu"
  "https://mirrors.edge.kernel.org/ubuntu"
)

mirror_is_available() {
  curl -fsS --head --max-time 10 "$1/dists/noble/InRelease" >/dev/null 2>&1
}

ubuntu_mirror="${UBUNTU_MIRROR:-}"
if [[ -z "$ubuntu_mirror" ]]; then
  for candidate in "${default_mirrors[@]}"; do
    if mirror_is_available "$candidate"; then
      ubuntu_mirror="$candidate"
      break
    fi
  done
fi
ubuntu_mirror="${ubuntu_mirror:-${default_mirrors[0]}}"
echo "Using Ubuntu apt mirror: ${ubuntu_mirror}"

# Replace the runner-provided mirror list wholesale. GitHub images have
# changed the default host before, so matching only azure.archive.ubuntu.com
# is not reliable.
if [[ -f /etc/apt/apt-mirrors.txt ]]; then
  printf '%s\n' "$ubuntu_mirror" | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
fi
for source in /etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources; do
  if [[ -f "$source" ]]; then
    sudo sed -i -E \
      "s#^URIs:.*#URIs: ${ubuntu_mirror}#; s#https?://[^ ]+/ubuntu#${ubuntu_mirror}#g" \
      "$source"
  fi
done

# Fail fast on a bad mirror instead of letting the job sit until timeout.
apt_options=(
  -o Acquire::Retries=3
  -o Acquire::http::Timeout=20
  -o Acquire::https::Timeout=20
  -o Acquire::ForceIPv4=true
)
sudo apt-get "${apt_options[@]}" update
sudo apt-get "${apt_options[@]}" install -y --no-install-recommends \
  gcc g++ clang libfontconfig-dev libwayland-dev \
  libwebkit2gtk-4.1-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libssl-dev libzstd-dev libvulkan1
