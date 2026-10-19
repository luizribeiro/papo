#!/usr/bin/env python3
import pathlib
import runpy
import tempfile
import unittest


MODULE = runpy.run_path(pathlib.Path(__file__).with_name("server-notes"))


class ServerNotesTests(unittest.TestCase):
    def test_failure_loader_rejects_unknown_kinds(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = pathlib.Path(temporary)
            for profile in MODULE["PROFILES"]:
                (directory / f"{profile}.toml").write_text("")
            (directory / "ergo-default.toml").write_text(
                "[registration]\n"
                "reason = 'wrong numeric'\n"
                "kind = 'implementation-bug'\n"
                "spec = 'https://example.invalid/spec'\n"
                "evidence = 'S: 400'\n"
                "server_version = '1.0'\n"
            )
            with self.assertRaisesRegex(ValueError, "unknown kind"):
                MODULE["load_failures"](directory)

    def test_patch_loader_requires_an_exact_index(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = pathlib.Path(temporary)
            (directory / "carried.patch").write_text("diff --git a/a b/a\n")
            index = directory / "fixture-patches.toml"
            index.write_text("")
            with self.assertRaisesRegex(ValueError, r"missing=\['carried.patch'\]"):
                MODULE["load_patches"](index, directory)

    def test_render_groups_kinds_and_describes_patches(self):
        entry = {
            "reason": "returns the wrong numeric",
            "kind": "server-bug",
            "spec": "https://example.invalid/spec",
            "evidence": "C: TEST | S: 400",
            "server_version": "1.0",
        }
        patch = {
            "profile": "ergo-default",
            "what": "Splits a reply.",
            "why": "The original is too long.",
            "evidence": "Before: 600 bytes; after: 400 bytes.",
            "server_bug": True,
        }
        output = MODULE["render"](
            {"ergo-default": {"registration": entry}},
            {"wire.patch": patch},
        )
        self.assertIn("### Server bugs", output)
        self.assertIn("`registration` (`1.0`)", output)
        self.assertIn("### Unimplemented features (none recorded)", output)
        self.assertIn("`nix/wire.patch`", output)
        self.assertIn("Server bug: **yes**", output)


if __name__ == "__main__":
    unittest.main()
