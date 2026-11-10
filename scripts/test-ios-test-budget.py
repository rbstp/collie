#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location('budget', Path(__file__).with_name('ios-test-budget.py'))
budget = importlib.util.module_from_spec(spec)
spec.loader.exec_module(budget)

VALUES = {'ui_tests': 2, 'ui_launches': 3, 'ui_seconds': 40.5, 'slowest_ui_test_seconds': 30.5, 'unit_seconds': 12}


def data(**changes):
    return {'metrics': dict(VALUES), 'ceilings': dict(VALUES), 'retried': [], **changes}


def artifact(content, name=budget.ARTIFACT_FILE):
    file = tempfile.NamedTemporaryFile(suffix='.zip')
    with zipfile.ZipFile(file, 'w', zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(name, content)
    file.flush()
    return file


class MeasureTests(unittest.TestCase):
    def test_counts_tests_launches_time_and_retries(self):
        def case(name, seconds, result='Passed', runs=()):
            return {
                'nodeType': 'Test Case', 'name': f'{name}()', 'nodeIdentifier': f'Suite/{name}()', 'nodeIdentifierURL': name,
                'result': result, 'durationInSeconds': seconds,
                'children': [{'nodeType': 'Repetition', 'name': f'Run {i}', 'result': r, 'durationInSeconds': seconds} for i, r in enumerate(runs)],
            }
        tests = {'testNodes': [{'nodeType': 'Test Plan', 'name': 'Collie', 'children': [
            {'nodeType': 'UI test bundle', 'name': 'CollieUITests', 'children': [{'nodeType': 'Test Suite', 'name': 'Suite', 'children': [
                case('testOne', 10), case('testRetried', 15.25, runs=('Failed', 'Passed')), case('testSkipped', 0, result='Skipped'),
            ]}]},
            {'nodeType': 'Unit test bundle', 'name': 'CollieTests', 'children': [case('testUnit', 5), case('unit(phase:)', 7)]},
        ]}]}
        launch = {'title': 'Open dev.rbstp.collie', 'childActivities': [{'title': 'Launch dev.rbstp.collie'}]}
        activities = {
            'testOne': {'testRuns': [{'activities': [launch, {'title': 'Tap'}, launch]}]},
            'testRetried': {'testRuns': [{'activities': [launch, launch]}, {'activities': [launch]}]},
        }
        measured = budget.measure(tests, activities.__getitem__)
        self.assertEqual(measured, {'metrics': VALUES, 'retried': ['CollieUITests/Suite/testRetried']})
        budget.validate({**measured, 'ceilings': VALUES})


class ValidateTests(unittest.TestCase):
    def test_accepts_only_the_measured_shape(self):
        budget.validate(data(retried=['CollieUITests/LandscapeTests/testSheetsKeepPromptAcrossRotation']))
        hostile = [
            [], 'text', {}, data(extra=1), data(metrics={**VALUES, 'more': 1}), data(ceilings={'ui_tests': 1}),
            data(metrics={**VALUES, 'ui_tests': '48'}), data(metrics={**VALUES, 'ui_tests': True}),
            data(metrics={**VALUES, 'ui_seconds': float('nan')}), data(metrics={**VALUES, 'ui_seconds': -1}),
            data(metrics={**VALUES, 'ui_seconds': 1e9}), data(metrics={**VALUES, 'ui_tests': [1]}),
            data(retried='CollieUITests/A/b'), data(retried=[1]), data(retried=['CollieUITests/A/b'] * 101),
            data(retried=['$(curl example.com)']), data(retried=['A/b`; rm -rf ~']), data(retried=['A/b | [x](http://example.com)']),
            data(retried=['A/<img src=x>']), data(retried=['A/' + 'b' * 81]), data(retried=['A/b\n| x |']), data(retried=['noSuite']),
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

    def test_without_a_baseline(self):
        summary, table, failed = budget.render(data())
        self.assertEqual((summary, failed), ('Within every enforced ceiling.', []))
        self.assertIn('| UI tests | n/a | 2 | n/a | 2 | pass |', table)


if __name__ == '__main__':
    unittest.main()
