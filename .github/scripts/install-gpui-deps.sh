#!/usr/bin/env bash
set -euo pipefail

# GitHub-hosted ubuntu-24.04 can route apt pool downloads through a slow
# azure.archive.ubuntu.com node. Prefer a stable public mirror so CI does not
# spend 20+ minutes retrying packages. UBUNTU_MIRROR can override this.
ubuntu_mirror="${UBUNTU_MIRROR:-https://mirrors.edge.kernel.org/ubuntu}"

for source in /etc/apt/apt-mirrors.txt /etc/apt/sources.list.d/ubuntu.sources; do
  if [[ -f "$source" ]]; then
    sudo sed -i -E "s#https?://(azure\.)?archive\.ubuntu\.com/ubuntu#${ubuntu_mirror}#g" "$source"
  fi
done

sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  gcc g++ clang libfontconfig-dev libwayland-dev \
  libwebkit2gtk-4.1-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libssl-dev libzstd-dev libvulkan1
