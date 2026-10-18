#!/usr/bin/env python3
import pathlib
import runpy
import unittest

MODULE = runpy.run_path(pathlib.Path(__file__).with_name("conformance-coverage"))

class CoverageTests(unittest.TestCase):
    def test_validation_requires_complete_current_metadata(self):
        profiles = MODULE["PROFILES"]
        statuses = {"one": {profile: "✓" for profile in profiles}}
        MODULE["validate"](["a", "b"], {"one": ["a"]}, {"b": "unobservable"}, statuses)
        cases = [
            (["a", "a"], {"one": ["a"]}, {}, "duplicate ids"),
            (["a"], {"one": ["unknown"]}, {}, "unknown checklist"),
            (["a"], {"one": []}, {"a": "waived"}, "without covers"),
            (["a"], {"one": ["a"]}, {"a": "stale"}, "still have waivers"),
            (["a", "b"], {"one": ["a"]}, {}, "without a scenario"),
            (["a"], {"two": ["a"]}, {}, "missing from conformance"),
            (["a"], {}, {"a": ""}, {}, "without reasons"),
        ]
        for checklist, declarations, waivers, *rest in cases:
            matrix, message = (statuses, rest[0]) if len(rest) == 1 else rest
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                MODULE["validate"](checklist, declarations, waivers, matrix)

    def test_render_combines_results_and_explains_waivers(self):
        MODULE["render"].__globals__["PROFILES"] = ("first", "second")
        MODULE["render"].__globals__["PROFILE_LABELS"] = ("First", "Second")
        statuses = {"one": {"first": "✓", "second": "—"},
                    "two": {"first": "⚠ [^1]", "second": "✗"}}
        output = MODULE["render"](["a", "b"], {"one": ["a"], "two": ["a"]},
                                  {"b": "linked | servers"}, statuses)
        self.assertIn("| `a` | `one`, `two` | ⚠ | ✗ |", output)
        self.assertIn("| `b` | Waiver: linked \\| servers | — | — |", output)

if __name__ == "__main__":
    unittest.main()
