# SPDX-License-Identifier: MIT
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("linux_elf", Path(__file__).with_name("linux-elf.py"))
linux_elf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(linux_elf)


class LinuxElfTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.binary = Path(directory.name) / "ptouch"
        self.binary.write_bytes(b"\x7fELF\x02\x01" + bytes(12) + b">\x00")

    def check(self, version="GLIBC_2.17", *, dynamic="", library="libudev.so.1"):
        report = (
            "[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]\n"
            f"Shared library: [{library}]\n{dynamic}\n"
            "Version needs section '.gnu.version_r' contains 1 entry:\n"
            f"  Name: {version}  Flags: none\n"
        )
        with patch.object(linux_elf.subprocess, "check_output", return_value=report):
            linux_elf.check_elf(self.binary, "x86_64-unknown-linux-gnu")

    def test_numeric_versions(self):
        self.check("GLIBC_2.9")
        self.check()
        with self.assertRaisesRegex(ValueError, "above 2.17"):
            self.check("GLIBC_2.43")

    def test_unsupported_versions_and_runtime_paths(self):
        with self.assertRaisesRegex(ValueError, "version namespace"):
            self.check("GLIBC_PRIVATE")
        with self.assertRaisesRegex(ValueError, "runtime search path"):
            self.check(dynamic="(RUNPATH) Library runpath: [/tmp/build]")

    def test_usb_backend_is_required(self):
        with self.assertRaisesRegex(ValueError, "missing the libudev"):
            self.check(library="libc.so.6")
