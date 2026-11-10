#!/usr/bin/env python3
"""The iOS test budget: measures a test run against Collie/Tests/budget.json and renders the table.

  measure <budget.json> <xcresult or the folder xcodebuild writes them to> <metrics.json>
      Run by the ios job in ci.yml. Writes the metrics, prints the table (to the job summary too) and fails over a ceiling.
  comment --pr <artifact.zip> [--baseline <artifact.zip>] --pr-sha <sha> [--baseline-sha <sha>] --conclusion <text>
      Run by ios-test-budget.yml, which holds a token that can write to pull requests: prints the comment's markdown.
      A pull request's artifact is whatever that pull request made it, so it is only ever read through load().
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import zipfile

MARKER = "<!-- ios-test-budget -->"
ARTIFACT_FILE = "ios-test-metrics.json"
METRICS = {
    "ui_tests": ("UI tests", ""),
    "ui_launches": ("App launches in the UI suite", ""),
    "ui_seconds": ("UI suite", " s"),
    "slowest_ui_test_seconds": ("Slowest UI test", " s"),
    "unit_seconds": ("Unit tests (CollieTests), summed", " s"),
}
# Reported, never enforced: a test's time includes a failed first attempt, so one 60 s idle stall and a passing retry
# would fail the job with no regression.
REPORTED = {"slowest_ui_test_seconds"}
ENFORCED = set(METRICS) - REPORTED
MAX_BYTES = 64 * 1024
MAX_VALUE = 1_000_000
MAX_RETRIED = 100
# The suite's longest name part is 83 characters.
NAME = re.compile(r"[A-Za-z0-9_]{1,200}(/[A-Za-z0-9_]{1,200}){1,3}")
SHA = re.compile(r"[0-9a-f]{40}")
CONCLUSION = re.compile(r"[a-z_]{1,20}")


def xcresulttool(bundle, *args):
    out = subprocess.run(
        ["xcrun", "xcresulttool", "get", "test-results", *args, "--path", str(bundle), "--compact"],
        check=True, capture_output=True,
    ).stdout
    return json.loads(out)


def cases(node, bundle=None):
    if node.get("nodeType", "").endswith("test bundle"):
        bundle = node["name"]
    if node.get("nodeType") == "Test Case":
        yield bundle, node
    else:
        for child in node.get("children", []):
            yield from cases(child, bundle)


def launches(activities):
    """App launches in a test's last run: a retried test is counted once, as it passed."""
    def count(activity):
        return activity.get("title", "").startswith("Launch ") + sum(count(c) for c in activity.get("childActivities", []))
    runs = activities.get("testRuns", [])
    return sum(count(a) for a in runs[-1].get("activities", [])) if runs else 0


def runs(node):
    """A test's retries and arguments, nested either way."""
    return [c for c in node.get("children", []) if c.get("nodeType") in ("Repetition", "Arguments")]


def seconds(node):
    """A retried or parameterized test's own duration is the mean of its runs: its cost is their sum."""
    return sum(seconds(c) for c in runs(node)) if runs(node) else node.get("durationInSeconds", 0)


def failed_once(node):
    return any(c.get("nodeType") == "Repetition" and c.get("result") == "Failed" or failed_once(c) for c in runs(node))


def measure(tests, activities):
    """`tests` is `xcresulttool get test-results tests`; `activities(test id)` is its `activities` for one test."""
    ui, unit, retried = [], [], []
    for node in tests.get("testNodes", []):
        for bundle, case in cases(node):
            if case.get("result") == "Skipped":
                continue
            if case.get("result") == "Passed" and failed_once(case):
                name = re.sub(r"\(.*", "", f"{bundle}/{case.get('nodeIdentifier', case['name'])}")
                retried.append(re.sub(r"[^A-Za-z0-9_/]", "_", name))
            if bundle == "CollieUITests":
                ui.append(case)
            elif bundle == "CollieTests":
                unit.append(case)
    times = [seconds(c) for c in ui]
    return {
        "metrics": {
            "ui_tests": len(ui),
            "ui_launches": sum(launches(activities(c["nodeIdentifierURL"])) for c in ui),
            "ui_seconds": round(sum(times), 1),
            "slowest_ui_test_seconds": round(max(times, default=0), 1),
            "unit_seconds": round(sum(seconds(c) for c in unit), 1),
        },
        "retried": sorted(retried),
    }


def validate(data):
    """Returns `data` if it is exactly what `measure` writes, and raises ValueError otherwise."""
    if not isinstance(data, dict) or set(data) != {"metrics", "ceilings", "retried"}:
        raise ValueError("unexpected keys")
    for part, keys in (("metrics", set(METRICS)), ("ceilings", ENFORCED)):
        values = data[part]
        if not isinstance(values, dict) or set(values) != keys:
            raise ValueError(f"unexpected {part}")
        for value in values.values():
            if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 <= value <= MAX_VALUE:
                raise ValueError(f"unexpected value in {part}")
    retried = data["retried"]
    if not isinstance(retried, list) or len(retried) > MAX_RETRIED:
        raise ValueError("unexpected retried")
    if not all(isinstance(name, str) and NAME.fullmatch(name) for name in retried):
        raise ValueError("unexpected test name")
    return data


def load(artifact):
    """The metrics in an artifact's zip, read in memory and bounded: nothing is extracted to disk."""
    try:
        with zipfile.ZipFile(artifact) as archive:
            info = archive.getinfo(ARTIFACT_FILE)
            if info.file_size > MAX_BYTES:
                raise ValueError("too large")
            with archive.open(info) as file:
                raw = file.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES:
            raise ValueError("too large")
        return validate(json.loads(raw))
    except (OSError, KeyError, RecursionError, zipfile.BadZipFile, UnicodeDecodeError) as error:
        raise ValueError(str(error)) from error


def number(value, unit):
    return f"{value:.0f}{unit}" if unit else f"{value:g}"


def render(data, baseline=None, column="This run"):
    """The summary line and the table. Every cell comes from a validated number or a name matching NAME."""
    rows, over = [], []
    for key, (label, unit) in METRICS.items():
        value, ceiling = data["metrics"][key], data["ceilings"].get(key)
        if ceiling is not None and value > ceiling:
            over.append(label)
        base, impact = "n/a", "n/a"
        if baseline:
            before = baseline["metrics"][key]
            base = number(before, unit)
            delta = value - before
            percent = f" ({delta / before:+.0%})" if before else ""
            impact = f"{'+' if delta > 0 else ''}{number(delta, unit)}{percent}"
        if ceiling is None:
            rows.append(f"| {label} | {base} | {number(value, unit)} | {impact} | not enforced | |")
        else:
            rows.append(f"| {label} | {base} | {number(value, unit)} | {impact} | {number(ceiling, unit)} | {'fail' if value > ceiling else 'pass'} |")
    names = lambda d: ", ".join(f"`{name}`" for name in d["retried"]) or "none"
    rows.append(f"| Passed only on the retry | {names(baseline) if baseline else 'n/a'} | {names(data)} | | not enforced | |")
    summary = f"Over the ceiling: {', '.join(over)}." if over else "Within every enforced ceiling."
    table = [f"| Metric | Main baseline | {column} | Impact | Ceiling | |", "|---|---|---|---|---|---|", *rows]
    return summary, table, over


def main():
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    measuring = commands.add_parser("measure")
    measuring.add_argument("budget", type=Path)
    measuring.add_argument("results", type=Path)
    measuring.add_argument("output", type=Path)
    commenting = commands.add_parser("comment")
    commenting.add_argument("--pr", type=Path, required=True)
    commenting.add_argument("--baseline", type=Path)
    commenting.add_argument("--pr-sha", required=True)
    commenting.add_argument("--baseline-sha", default="")
    commenting.add_argument("--conclusion", required=True)
    args = parser.parse_args()

    if args.command == "measure":
        bundles = [args.results] if args.results.suffix == ".xcresult" else sorted(args.results.glob("*.xcresult"), key=lambda p: p.stat().st_mtime)
        if not bundles:
            sys.exit(f"no result bundle in {args.results}: the tests did not run")
        bundle = bundles[-1]
        data = measure(xcresulttool(bundle, "tests"), lambda test: xcresulttool(bundle, "activities", "--test-id", test))
        data["ceilings"] = json.loads(args.budget.read_text())
        validate(data)
        args.output.write_text(json.dumps(data, indent=2) + "\n")
        summary, table, over = render(data)
        text = "\n".join(["### iOS test budget", "", summary, "", *table]) + "\n"
        print(text)
        if "GITHUB_STEP_SUMMARY" in os.environ:
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as file:
                file.write(text)
        for name in data["retried"]:
            print(f"::warning title=Test passed only on the retry::{name}")
        if over:
            sys.exit(f"{summary} Make the tests cheaper, or raise the ceiling in {args.budget} and say why in the pull request.")
        return

    if not SHA.fullmatch(args.pr_sha) or not CONCLUSION.fullmatch(args.conclusion) or (args.baseline_sha and not SHA.fullmatch(args.baseline_sha)):
        sys.exit("unexpected commit or conclusion")
    try:
        data = load(args.pr)
    except ValueError as error:
        sys.exit(f"the pull request's metrics are not usable: {error}")
    baseline = None
    if args.baseline and args.baseline_sha:
        try:
            baseline = load(args.baseline)
        except ValueError as error:
            print(f"the baseline's metrics are not usable: {error}", file=sys.stderr)
    summary, table, _ = render(data, baseline, "This PR")
    print("\n".join([
        MARKER, "### iOS test budget", "", summary, "", *table, "",
        f"Baseline: {args.baseline_sha if baseline else 'n/a'}. Pull request: {args.pr_sha}. `ci` concluded: {args.conclusion}.",
    ]))


if __name__ == "__main__":
    main()
