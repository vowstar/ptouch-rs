# SPDX-License-Identifier: MIT
import importlib.util
from pathlib import Path
import plistlib
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("macos_package", Path(__file__).with_name("macos-package.py"))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class MacPackageTest(unittest.TestCase):
    def test_rejects_metadata_that_does_not_match_release(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / package.APP_NAME
            (app / "Contents").mkdir(parents=True)
            with (app / "Contents/Info.plist").open("wb") as stream:
                plistlib.dump({"CFBundleIdentifier": "another-app"}, stream)
            with self.assertRaisesRegex(ValueError, "metadata"):
                package.check_bundle(app, "1.2.3")

    def test_rejects_files_outside_the_app_and_symlinks(self):
        for name, mode in (("../outside", 0), ("/absolute", 0), ("other.app/Contents/file", 0),
                           ("ptouch-gui.app/../../outside", 0),
                           ("ptouch-gui.app/Contents/link", 0o120777 << 16)):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "app.zip"
                with zipfile.ZipFile(path, "w") as archive:
                    entry = zipfile.ZipInfo(name)
                    entry.external_attr = mode
                    archive.writestr(entry, b"file")
                with self.assertRaisesRegex(ValueError, "archive member"):
                    package.check_archive(path)

    def test_accepts_app_and_ditto_metadata_only(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "app.zip"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("ptouch-gui.app/Contents/Info.plist", b"plist")
                archive.writestr("__MACOSX/ptouch-gui.app/Contents/._Info.plist", b"metadata")
            package.check_archive(path)
