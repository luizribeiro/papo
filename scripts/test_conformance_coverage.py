#!/usr/bin/env python3
import pathlib
import runpy
import unittest

MODULE = runpy.run_path(pathlib.Path(__file__).with_name("conformance-coverage"))

class CoverageTests(unittest.TestCase):
    def test_loads_describe_records_from_libtest_output(self):
        declarations = MODULE["load_declarations"]([
            'test scenarios::connection::connect ... { "type": "covers", '
            '"name": "connect", "behaviors": ["base.connreg.nick-user"] }',
            "ok",
        ])
        self.assertEqual(declarations, {"connect": ["base.connreg.nick-user"]})

    def test_validation_requires_complete_current_metadata(self):
        profiles = MODULE["PROFILES"]
        statuses = {"one": {profile: "✓" for profile in profiles}}
        MODULE["validate"](
            ["a", "b"], {"one": ["a"]}, {}, {"b": "unobservable"}, statuses
        )
        cases = [
            (["a", "a"], {"one": ["a"]}, {}, {}, "duplicate ids"),
            (["a"], {"one": ["unknown"]}, {}, {}, "unknown checklist"),
            (["a"], {"one": []}, {}, {"a": "waived"}, "without covers"),
            (["a"], {}, {"security::one": []}, {"a": "waived"}, "without covers"),
            (["a"], {"one": ["a"]}, {}, {"a": "stale"}, "still have waivers"),
            (["a", "b"], {"one": ["a"]}, {}, {}, "without a scenario"),
            (["a"], {"two": ["a"]}, {}, {}, "missing from conformance"),
            (["a"], {}, {}, {"a": ""}, {}, "without reasons"),
        ]
        for checklist, declarations, wire, waivers, *rest in cases:
            matrix, message = (statuses, rest[0]) if len(rest) == 1 else rest
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                MODULE["validate"](checklist, declarations, wire, waivers, matrix)

    def test_validation_counts_wire_tests_as_coverage(self):
        statuses = {"one": {profile: "✓" for profile in MODULE["PROFILES"]}}
        MODULE["validate"](
            ["live", "security"],
            {"one": ["live"]},
            {"security::proof": ["security"]},
            {},
            statuses,
        )

    def test_render_combines_results_and_explains_waivers(self):
        MODULE["render"].__globals__["PROFILES"] = ("first", "second")
        MODULE["render"].__globals__["PROFILE_LABELS"] = ("First", "Second")
        statuses = {"one": {"first": "✓", "second": "—"},
                    "two": {"first": "⚠ [^1]", "second": "✗"}}
        output = MODULE["render"](["a", "b"], {"one": ["a"], "two": ["a"]}, {},
                                  {"b": "linked | servers"}, statuses)
        self.assertIn("| `a` | `one`, `two` | ⚠ | ✗ |", output)
        self.assertIn("| `b` | Waiver: linked \\| servers | — | — |", output)

    def test_render_marks_wire_tests_as_profile_independent(self):
        MODULE["render"].__globals__["PROFILES"] = ("first", "second")
        MODULE["render"].__globals__["PROFILE_LABELS"] = ("First", "Second")
        output = MODULE["render"](
            ["security"], {}, {"security::proof": ["security"]}, {}, {}
        )
        self.assertIn(
            "| `security` | Wire test `security::proof` | ✓ | ✓ |", output
        )

if __name__ == "__main__":
    unittest.main()
