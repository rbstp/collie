#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import socket
import sys
import tempfile
import unittest
from unittest.mock import patch

python_app = Path(sys.prefix) / 'Resources/Python.app/Contents/MacOS/Python'
PYTHON = str(python_app) if python_app.is_file() else sys.executable

spec = importlib.util.spec_from_file_location('vm_simulator', Path(__file__).with_name('vm-simulator.py'))
vm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vm)


class IsolationTests(unittest.TestCase):
    def test_unknown_guest_is_never_adopted_or_mutated(self):
        vm.STATE = {}
        response = type('Result', (), {'stdout': json.dumps([{'Name': vm.GUEST}]).encode()})()
        with patch.object(vm, 'tart', return_value=response) as tart:
            with self.assertRaisesRegex(RuntimeError, 'unrecorded guest'):
                vm.boot(Path('/unused'))
            self.assertEqual(tart.call_count, 1)

    def test_recorded_simulator_with_changed_name_is_not_touched(self):
        vm.STATE = {'simulator': 'test-id'}
        response = type('Result', (), {'stdout': json.dumps({'devices': {'runtime': [{'udid': 'test-id', 'name': 'Personal phone'}]}}).encode()})()
        with patch.object(vm, 'save'), patch.object(vm, 'run', return_value=response) as run:
            with self.assertRaisesRegex(RuntimeError, 'unexpected name'):
                vm.stop()
            self.assertEqual(run.call_count, 1)

    def test_owned_crashed_parent_process_can_be_recovered(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(vm,'ROOT',Path(directory)):
            vm.STATE = {'owner':'collie-vm-simulator-v1'}
            process = vm.spawn([PYTHON,'-c','import time; time.sleep(60)'],Path(directory)/'owned.log')
            try:
                self.assertIsNone(process.poll())
                self.assertEqual(vm.STATE['processes'][0]['pid'],process.pid)
                vm.stop_processes()
                self.assertLess(process.wait(timeout=3),0)
            finally:
                if process.poll() is None: process.kill();process.wait()
                process.stdin.close()
                vm.CHILDREN.remove(process)

    def test_term_resistant_owned_process_is_killed_before_forgetting_it(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(vm,'ROOT',Path(directory)):
            vm.STATE = {'owner':'collie-vm-simulator-v1'}
            ready = Path(directory)/'ready'
            code = f"import signal,time,pathlib;signal.signal(signal.SIGTERM,signal.SIG_IGN);pathlib.Path({str(ready)!r}).touch();time.sleep(60)"
            process = vm.spawn([PYTHON,'-c',code],Path(directory)/'owned.log')
            try:
                vm.wait_for(ready.exists,'owned signal handler',3)
                vm.stop_processes()
                self.assertEqual(process.wait(timeout=3),-9)
                self.assertNotIn('processes',vm.STATE)
            finally:
                if process.poll() is None: process.kill();process.wait()
                process.stdin.close()
                vm.CHILDREN.remove(process)

    def test_reused_pid_is_never_signaled(self):
        vm.STATE = {'processes': [{'pid':12345,'fingerprint':'old owned test'}]}
        with patch.object(vm,'process_fingerprint',return_value='different process'), patch.object(vm,'save'), patch.object(vm.os,'killpg') as kill:
            vm.stop_processes()
            kill.assert_not_called()
        self.assertNotIn('processes',vm.STATE)

    def test_private_copy_never_overwrites_or_follows_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / 'owner'
            target.write_text('preserve')
            link = root / 'test'
            link.symlink_to(target)
            with self.assertRaises(FileExistsError):
                vm.private(link, 'test identity')
            self.assertEqual(target.read_text(), 'preserve')

    def test_occupied_loopback_fails_without_killing_its_owner(self):
        with socket.socket() as server:
            try:
                server.bind(('127.0.0.1', 8788))
            except OSError:
                self.skipTest('test port already occupied')
            server.listen()
            with self.assertRaisesRegex(RuntimeError, 'occupied'):
                vm.require_ports()
            with socket.create_connection(server.getsockname(), timeout=1):
                pass


if __name__ == '__main__':
    unittest.main()
