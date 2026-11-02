# SPDX-License-Identifier: MIT
import importlib.util
from pathlib import Path
import re
import unittest


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name.replace("_", "-") + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleaseDownloadsTest(unittest.TestCase):
    def test_download_links_match_the_published_asset_names(self):
        version = "1.2.3"
        notes = load("release_downloads").download_notes(version)
        assets = set(load("release_assets").asset_map(version).values())
        links = re.findall(r"https://github.com/vowstar/ptouch-rs/releases/download/v([^/)]+)/([^\s)]+)", notes)
        self.assertEqual(len(links), 8)
        for tag, filename in links:
            self.assertEqual(tag, version)
            self.assertIn(filename, assets)
        editor, advanced = notes.split("<details>")
        self.assertIn("not notarized", editor)
        self.assertIn("/ptouch-windows-arm64.zip", editor)
        self.assertNotIn("/ptouch-windows-arm64.exe", editor)
        self.assertIn("/ptouch-windows-arm64.exe", advanced)
