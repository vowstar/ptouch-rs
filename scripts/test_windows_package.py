# SPDX-License-Identifier: MIT
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("windows_package", Path(__file__).with_name("windows-package.py"))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class WindowsPackageTest(unittest.TestCase):
    def test_both_downloads_keep_only_the_editor_at_the_top_level(self):
        with tempfile.TemporaryDirectory(prefix="ptouch package 启动 ") as temporary:
            root = Path(temporary)
            for name in ("ptouch.exe", "ptouch-gui.exe"):
                (root / name).write_bytes(name.encode())
            for target, arch in package.TARGETS.items():
                archive = package.build(root, root / "output", target)
                self.assertEqual(archive.name, f"ptouch-windows-{arch}.zip")
                with zipfile.ZipFile(archive) as bundle:
                    self.assertEqual(set(bundle.namelist()), {"ptouch-gui.exe", "cli/ptouch.exe"})
                    for member, source in package.MEMBERS.items():
                        self.assertEqual(bundle.read(member), (root / source).read_bytes())

    def test_missing_cli_cannot_produce_an_incomplete_download(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "ptouch-gui.exe").write_bytes(b"editor")
            with self.assertRaisesRegex(ValueError, "ptouch.exe"):
                package.build(root, root / "output", "aarch64-pc-windows-msvc")
            self.assertFalse((root / "output").exists())
