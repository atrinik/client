#!/usr/bin/env bash
set -euo pipefail

# SDL is compiled from its Cargo-locked source. These host headers enable the
# Linux desktop backends without selecting an ambient SDL library.
sudo apt-get update
sudo apt-get install --no-install-recommends --yes \
  libasound2-dev \
  libdbus-1-dev \
  libdecor-0-dev \
  libdrm-dev \
  libegl1-mesa-dev \
  libgbm-dev \
  libgl1-mesa-dev \
  libgles2-mesa-dev \
  libibus-1.0-dev \
  libpipewire-0.3-dev \
  libpulse-dev \
  libsndio-dev \
  libudev-dev \
  liburing-dev \
  libwayland-dev \
  libx11-dev \
  libxcursor-dev \
  libxext-dev \
  libxfixes-dev \
  libxi-dev \
  libxkbcommon-dev \
  libxrandr-dev \
  libxss-dev \
  libxtst-dev
