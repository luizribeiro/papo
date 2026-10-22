#!/usr/bin/env python3
"""Summarize status records from an ordinary papo-conformance test run."""

import json
import re
import sys

SCENARIO_EVENTS = {"passed", "failed", "ignored", "expected-failure", "unexpected-pass"}
TEST_START = re.compile(r"^test (scenarios::\S+) \.\.\.")


def records(lines):
    """Yield JSON records embedded in libtest output lines."""
    for line in lines:
        marker = line.find('{ "type":')
        if marker != -1:
            yield json.loads(line[marker:])


def summarize(lines):
    """Return scenario statuses from papo scenario records."""
    return summarize_with_evidence(lines)[0]


def summarize_with_evidence(lines):
    """Return scenario statuses and their recorded wire evidence."""
    statuses = {}
    evidence = {}
    for record in records(lines):
        name = record.get("name")
        event = record.get("event")
        if record.get("type") == "evidence":
            evidence[name] = record["value"]
        elif record.get("type") == "scenario":
            if event not in SCENARIO_EVENTS:
                raise ValueError(f"unknown scenario status {event!r} for {name}")
            if name in statuses:
                raise ValueError(f"scenario {name!r} printed more than one status")
            if event == "ignored" and record.get("reason"):
                event = f"ignored ({record['reason']})"
            statuses[name] = event
    return statuses, evidence


def missing_scenario_statuses(lines, statuses):
    """Return generated scenario tests that emitted no status record."""
    tests = set()
    for line in lines:
        match = TEST_START.match(line)
        if match and "::tests::" not in match.group(1):
            tests.add(match.group(1).rsplit("::", 1)[-1])
    return sorted(tests - set(statuses))


def render(statuses):
    """Render statuses in stable scenario-name order."""
    return "".join(f"{name} -> {statuses[name]}\n" for name in sorted(statuses))


if __name__ == "__main__":
    sys.stdout.write(render(summarize(sys.stdin)))
