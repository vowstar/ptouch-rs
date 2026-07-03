#!/bin/sh
# Post-install: reload udev so the USB permission rule applies without a replug,
# and refresh the icon cache so the desktop entry shows its icon.
set -e

if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules || true
    udevadm trigger --subsystem-match=usb || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -q /usr/share/icons/hicolor || true
fi

exit 0
