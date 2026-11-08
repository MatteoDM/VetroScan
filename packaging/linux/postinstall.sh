#!/bin/sh
# After installing (deb, rpm, pacman): reload the udev rules and apply 70-hp3770.rules to a
# scanner already plugged in, so it opens without unplugging it. Never fails the install:
# containers and chroots have no running udev.
if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload >/dev/null 2>&1 || true
  udevadm trigger --subsystem-match=usb --attr-match=idVendor=03f0 --attr-match=idProduct=2505 \
    >/dev/null 2>&1 || true
fi
