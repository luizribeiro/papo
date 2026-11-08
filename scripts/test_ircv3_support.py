#!/usr/bin/env python3
import pathlib
import runpy
import tempfile
import unittest


MODULE = runpy.run_path(pathlib.Path(__file__).with_name("ircv3-support"))


class Ircv3SupportTests(unittest.TestCase):
    def test_catalog_requires_reasons_for_unsupported_specs(self):
        with tempfile.TemporaryDirectory() as temporary:
            catalog = pathlib.Path(temporary) / "catalog.toml"
            catalog.write_text(
                "[[group]]\n"
                "title = 'Deprecated or unsupported'\n"
                "specs = [{ name = 'STARTTLS', url = 'https://example.invalid', "
                "wire = 'tls' }]\n"
            )
            with self.assertRaisesRegex(ValueError, "STARTTLS has no unsupported reason"):
                MODULE["load_catalog"](catalog)

    def test_unsupported_group_has_notes_in_place_of_support_columns(self):
        groups = [
            {
                "title": "Deprecated or unsupported",
                "specs": [
                    {
                        "name": "STARTTLS",
                        "url": "https://example.invalid/starttls",
                        "wire": "tls",
                        "reason": "Deprecated by IRCv3.",
                    }
                ],
            }
        ]
        output = MODULE["render"](groups, {}, set(), set())
        self.assertIn(
            "| Specification | Capability, tag, or token | Notes |", output
        )
        self.assertIn("| `tls` | Deprecated by IRCv3. |", output)
        self.assertNotIn("| Support | papo API | Live conformance |", output)

    def test_unsupported_notes_mark_unmerged_upstream_proposals(self):
        proposal = {
            "url": "https://github.com/ircv3/ircv3-specifications/pull/496",
            "reason": "Not yet implemented.",
        }
        published = {
            "url": "https://ircv3.net/specs/deprecated/tls",
            "reason": "Deprecated in favor of direct TLS.",
        }
        self.assertEqual(
            MODULE["unsupported_notes"](proposal),
            "Unmerged upstream proposal (pull request 496). Not yet implemented.",
        )
        self.assertEqual(
            MODULE["unsupported_notes"](published), "Deprecated in favor of direct TLS."
        )


if __name__ == "__main__":
    unittest.main()
