#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import re
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location('budget', Path(__file__).with_name('ios-test-budget.py'))
budget = importlib.util.module_from_spec(spec)
spec.loader.exec_module(budget)

ROOT = Path(__file__).resolve().parents[1]
VALUES = {'ui_tests': 2, 'ui_launches': 3, 'ui_seconds': 40.5, 'slowest_ui_test_seconds': 30.5, 'unit_seconds': 16}
CEILINGS = {key: value for key, value in VALUES.items() if key != 'slowest_ui_test_seconds'}


def data(**changes):
    return {'metrics': dict(VALUES), 'ceilings': dict(CEILINGS), 'retried': [], **changes}


def case(identifier, seconds, result='Passed', runs=(), arguments=()):
    """A test case as xcresulttool reports it: a retried or parameterized test's own duration is the mean of its runs."""
    children = [{'nodeType': 'Repetition', 'name': f'Run {i}', 'result': r, 'durationInSeconds': seconds} for i, r in enumerate(runs)]
    children += [{'nodeType': 'Arguments', 'name': name, 'durationInSeconds': t, 'children': c} for name, t, c in arguments]
    return {
        'nodeType': 'Test Case', 'name': identifier.split('/')[-1], 'nodeIdentifier': identifier, 'nodeIdentifierURL': identifier,
        'result': result, 'durationInSeconds': seconds, 'children': children,
    }


def tree(ui=(), unit=()):
    return {'testNodes': [{'nodeType': 'Test Plan', 'name': 'Collie', 'children': [
        {'nodeType': 'UI test bundle', 'name': 'CollieUITests', 'children': [{'nodeType': 'Test Suite', 'name': 'Suite', 'children': list(ui)}]},
        {'nodeType': 'Unit test bundle', 'name': 'CollieTests', 'children': list(unit)},
    ]}]}


def artifact(content, name=budget.ARTIFACT_FILE):
    file = tempfile.NamedTemporaryFile(suffix='.zip')
    with zipfile.ZipFile(file, 'w', zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(name, content)
    file.flush()
    return file


class MeasureTests(unittest.TestCase):
    def test_counts_tests_launches_time_and_retries(self):
        retried = [{'nodeType': 'Repetition', 'name': 'First Run', 'result': 'Failed', 'durationInSeconds': 2},
                   {'nodeType': 'Repetition', 'name': 'Retry 1', 'result': 'Passed', 'durationInSeconds': 1}]
        retried_ui = case('Suite/testRetried()', 35.25)
        retried_ui['children'] = [{'nodeType': 'Repetition', 'name': 'First Run', 'result': 'Failed', 'durationInSeconds': 40},
                                  {'nodeType': 'Repetition', 'name': 'Retry 1', 'result': 'Passed', 'durationInSeconds': 30.5}]
        # What Xcode 27.2 writes when argument "1" of a parameterized test fails once: the argument and its test stay marked
        # failed, the argument has no duration of its own, and the passing argument has no attempts.
        retried_argument = case('retriedArgument(x:)', 1.67, result='Failed', arguments=[('1', None, retried), ('2', 3, [])])
        retried_argument['children'][0]['result'] = 'Failed'
        retried_argument['children'][1]['result'] = 'Passed'
        tests = tree(
            ui=[case('Suite/testOne()', 10), retried_ui, case('Suite/testSkipped()', 0, result='Skipped')],
            unit=[
                case('Suite/testUnit()', 5),
                case('unit(phase:)', 3.5, arguments=[('.a', 3, []), ('.b', 4, [])]),
                retried_argument,
            ],
        )
        launch = {'title': 'Open dev.rbstp.collie', 'childActivities': [{'title': 'Launch dev.rbstp.collie'}]}
        activities = {
            'Suite/testOne()': {'testRuns': [{'activities': [launch, {'title': 'Tap'}, launch]}]},
            'Suite/testRetried()': {'testRuns': [{'activities': [launch, launch]}, {'activities': [launch]}]},
        }
        measured = budget.measure(tests, activities.__getitem__)
        self.assertEqual(measured, {'metrics': VALUES, 'retried': ['CollieTests/retriedArgument', 'CollieUITests/Suite/testRetried']})
        budget.validate({**measured, 'ceilings': CEILINGS})

    def test_an_expected_failure_attempt_counts_and_is_no_retry(self):
        expected = case('Suite/testExpected()', 1.5)
        expected['children'] = [{'nodeType': 'Repetition', 'name': 'First Run', 'result': 'Expected Failure', 'durationInSeconds': 2},
                                {'nodeType': 'Repetition', 'name': 'Retry 1', 'result': 'Passed', 'durationInSeconds': 1}]
        measured = budget.measure(tree(ui=[expected]), lambda _: {'testRuns': []})
        self.assertEqual(measured['metrics']['ui_seconds'], 3)
        self.assertEqual(measured['retried'], [])

    def test_a_test_that_passed_on_the_retry_counts_only_its_passing_attempt(self):
        attempts = [{'nodeType': 'Repetition', 'name': 'First Run', 'result': 'Failed', 'durationInSeconds': 115.2},
                    {'nodeType': 'Repetition', 'name': 'Retry 1', 'result': 'Passed', 'durationInSeconds': 101.1}]
        retried = case('LandscapeTests/testRetried()', 108.15)
        retried['children'] = attempts
        measured = budget.measure(tree(ui=[retried, case('LandscapeTests/testOther()', 20)]), lambda _: {'testRuns': []})
        self.assertEqual(measured['metrics']['ui_seconds'], 121.1)
        self.assertEqual(measured['metrics']['slowest_ui_test_seconds'], 101.1)
        self.assertEqual(measured['retried'], ['CollieUITests/LandscapeTests/testRetried'])

    def test_the_suites_longest_names_pass_on_the_retry_path(self):
        identifiers = []
        for bundle in ('CollieUITests', 'CollieTests'):
            for file in sorted((ROOT / 'Collie/Tests' / bundle).glob('*.swift')):
                suite = None
                for line in file.read_text().splitlines():
                    if found := re.search(r'class (\w+): XCTestCase', line):
                        suite = found.group(1)
                    if found := re.search(r'func (test\w+)\(', line):
                        identifiers.append((bundle, f'{suite}/{found.group(1)}()'))
                    elif found := re.search(r'^func (\w+)\(', line):
                        identifiers.append((bundle, f'{found.group(1)}()'))
        longest = sorted(identifiers, key=lambda i: -max(len(part) for part in i[1].split('/')))[:budget.MAX_RETRIED]
        self.assertGreater(max(len(part) for _, i in longest for part in i.split('/')), 80, 'the scan found the long names')
        tests = tree(
            ui=[case(i, 1, runs=('Failed', 'Passed')) for bundle, i in longest if bundle == 'CollieUITests'],
            unit=[case(i, 1, runs=('Failed', 'Passed')) for bundle, i in longest if bundle == 'CollieTests'],
        )
        measured = budget.measure(tests, lambda _: {'testRuns': []})
        self.assertEqual(len(measured['retried']), len(longest))
        budget.validate({**measured, 'ceilings': CEILINGS})


class ValidateTests(unittest.TestCase):
    def test_accepts_only_the_measured_shape(self):
        budget.validate(data(retried=['CollieUITests/LandscapeTests/testSheetsKeepPromptAcrossRotation']))
        hostile = [
            [], 'text', {}, data(extra=1), data(metrics={**VALUES, 'more': 1}), data(ceilings={'ui_tests': 1}), data(ceilings=dict(VALUES)),
            data(metrics={**VALUES, 'ui_tests': '48'}), data(metrics={**VALUES, 'ui_tests': True}),
            data(metrics={**VALUES, 'ui_seconds': float('nan')}), data(metrics={**VALUES, 'ui_seconds': -1}),
            data(metrics={**VALUES, 'ui_seconds': 1e9}), data(metrics={**VALUES, 'ui_tests': [1]}),
            data(retried='CollieUITests/A/b'), data(retried=[1]), data(retried=['CollieUITests/A/b'] * 101),
            data(retried=['$(curl example.com)']), data(retried=['A/b`; rm -rf ~']), data(retried=['A/b | [x](http://example.com)']),
            data(retried=['A/<img src=x>']), data(retried=['A/' + 'b' * 201]), data(retried=['A/b\n| x |']), data(retried=['noSuite']),
        ]
        for value in hostile:
            with self.assertRaises(ValueError, msg=repr(value)[:80]):
                budget.validate(value)

    def test_reads_a_bounded_file_from_the_zip(self):
        with artifact(json.dumps(data())) as file:
            self.assertEqual(budget.load(file.name), data())
        padded = json.dumps(data()) + ' ' * budget.MAX_BYTES
        unusable = [artifact(padded), artifact('{"metrics":'), artifact('[' * 50_000), artifact(json.dumps(data()), name='../other.json'), artifact(b'\xff\xfe')]
        for file in unusable:
            with file, self.assertRaises(ValueError):
                budget.load(file.name)
        with tempfile.NamedTemporaryFile() as file:
            file.write(b'not a zip')
            file.flush()
            with self.assertRaises(ValueError):
                budget.load(file.name)


class RenderTests(unittest.TestCase):
    def test_marks_the_rows_over_their_ceiling(self):
        over = data(metrics={**VALUES, 'ui_tests': 3, 'ui_seconds': 41}, retried=['CollieUITests/A/testB'])
        summary, table, failed = budget.render(over, data(metrics={**VALUES, 'ui_tests': 4}))
        self.assertEqual(summary, 'Over the ceiling: UI tests, UI suite.')
        self.assertEqual(failed, ['UI tests', 'UI suite'])
        self.assertIn('| UI tests | 4 | 3 | -1 (-25%) | 2 | fail |', table)
        self.assertIn('| UI suite | 40 s | 41 s | +0 s (+1%) | 40 s | fail |', table)
        self.assertIn('| App launches in the UI suite | 3 | 3 | 0 (+0%) | 3 | pass |', table)
        self.assertIn('| Passed only on the retry | none | `CollieUITests/A/testB` | | not enforced | |', table)

    def test_without_a_baseline_and_with_a_slow_retry(self):
        summary, table, failed = budget.render(data(metrics={**VALUES, 'slowest_ui_test_seconds': 500}))
        self.assertEqual((summary, failed), ('Within every enforced ceiling.', []))
        self.assertIn('| UI tests | n/a | 2 | n/a | 2 | pass |', table)
        self.assertIn('| Slowest UI test | n/a | 500 s | n/a | not enforced | |', table)


if __name__ == '__main__':
    unittest.main()
