#!/usr/bin/env python3
"""Tests for the conformance status summarizer."""

import io
import unittest

from summary import missing_scenario_statuses, render, summarize, summarize_with_evidence


SAMPLE = """\
running 3 tests
test scenarios::connection::connect ... { "type": "evidence", "name": "connect", "value": "longest S: 487 bytes, CAP" }
{ "type": "scenario", "name": "connect", "event": "passed", "reason": null }
ok
test scenarios::connection::websocket ... { "type": "scenario", "name": "websocket", "event": "ignored", "reason": "PAPO_TEST_WS is not set up" }
ok
test scenarios::metadata::metadata ... { "type": "scenario", "name": "metadata", "event": "expected-failure", "reason": "server does not implement metadata" }
ok
"""


class SummaryTests(unittest.TestCase):
    def test_captured_status_records_preserve_scenario_outcomes(self):
        output = render(summarize(io.StringIO(SAMPLE)))
        self.assertEqual(
            output,
            "connect -> passed\n"
            "metadata -> expected-failure\n"
            "websocket -> ignored (PAPO_TEST_WS is not set up)\n",
        )

    def test_empty_input_has_no_scenarios(self):
        self.assertEqual(summarize(io.StringIO("")), {})

    def test_keeps_scenario_evidence_separate_from_status(self):
        statuses, evidence = summarize_with_evidence(io.StringIO(SAMPLE))
        self.assertEqual(statuses["connect"], "passed")
        self.assertEqual(evidence["connect"], "longest S: 487 bytes, CAP")

    def test_detects_missing_and_duplicate_status_records(self):
        lines = SAMPLE.splitlines()
        self.assertEqual(missing_scenario_statuses(lines, summarize(lines)), [])
        missing = lines + ["test scenarios::presence::away_notify ... ok"]
        self.assertEqual(missing_scenario_statuses(missing, summarize(missing)), ["away_notify"])
        with self.assertRaisesRegex(ValueError, "more than one status"):
            summarize(lines + ['{ "type": "scenario", "name": "connect", "event": "passed" }'])


if __name__ == "__main__":
    unittest.main()
