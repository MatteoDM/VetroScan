#!/bin/sh
# After removing (deb, rpm, pacman): reload the udev rules, now without 70-hp3770.rules.
# Never fails the removal.
if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload >/dev/null 2>&1 || true
fi
