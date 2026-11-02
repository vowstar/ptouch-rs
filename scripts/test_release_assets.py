# SPDX-License-Identifier: MIT
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("release_assets", Path(__file__).with_name("release-assets.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseAssetsTest(unittest.TestCase):
    def test_complete_release_includes_both_windows_architectures_and_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for source in release.asset_map("0.8.3"):
                path = root / "artifacts" / source
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(source.encode())
            release.assemble(root / "artifacts", root / "release", "0.8.3")
            for name in (
                "ptouch-windows-arm64.exe", "ptouch-gui-windows-arm64.exe", "ptouch-windows-amd64.exe",
                "ptouch-linux-arm64", "ptouch-gui-macos-arm64.app.zip",
                "ptouch-windows-arm64.zip", "ptouch-windows-amd64.zip",
                "usbprint-probe-windows-arm64.exe", "usbprint-probe-windows-amd64.exe",
            ):
                self.assertTrue((root / "release" / name).is_file())
            for line in (root / "release" / "SHA256SUMS").read_text().splitlines():
                digest, name = line.split("  ")
                self.assertEqual(digest, hashlib.sha256((root / "release" / name).read_bytes()).hexdigest())

    def test_missing_arm64_stops_before_creating_release(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for source in release.asset_map("0.8.3"):
                if "windows-arm64" in source:
                    continue
                path = root / "artifacts" / source
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"artifact")
            with self.assertRaisesRegex(ValueError, "windows-arm64"):
                release.assemble(root / "artifacts", root / "release", "0.8.3")
            self.assertFalse((root / "release").exists())
