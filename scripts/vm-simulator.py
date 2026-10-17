#!/usr/bin/env python3
import argparse
import base64
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.request
import uuid

REPO = Path(__file__).resolve().parents[1]
COMMON = Path(subprocess.check_output(['git', 'rev-parse', '--path-format=absolute', '--git-common-dir'], cwd=REPO, text=True).strip())
ROOT = COMMON.parent.parent / '.codex/vms/collie-374'
TART_VERSION = '2.41.1'
TART_SHA = 'e680a734febccfce7d4e910c1f7f13fc3b00f19a181c945e2d746de3214c32c2'
IMAGE = 'ghcr.io/cirruslabs/macos-golden-gate-base@sha256:5302a9ea6cd68c059d00fa253fe75c8ebdf4e0dc8c45d4e136ecc6e7a4c02b70'
BASE = 'collie-374-baseline'
GUEST = 'collie-374-test'
READY = 'collie-374-desktop'
DRIVER_VERSION = '0.36.0'
DRIVER_SHA = '4a0e229922a1ac6d6f59152e0653541920951afa89a3007e4ff5ffc024eae7a8'
DRIVER = '/Applications/CuaDriver.app/Contents/MacOS/cua-driver'
TART = ROOT / 'tools/tart.app/Contents/MacOS/tart'
ENV = {**os.environ, 'TART_HOME': str(ROOT / 'tart'), 'TART_NO_AUTO_PRUNE': '1'}
CHILDREN = []
STATE = {}


def run(args, *, cwd=REPO, env=None, data=None, check=True, timeout=180):
    result = subprocess.run([str(a) for a in args], cwd=cwd, env=env, input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f'{args[0]} failed ({result.returncode}): {result.stderr.decode(errors="replace")[-3000:]}')
    return result


def private(path, data):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as file:
        file.write(data if isinstance(data, bytes) else data.encode())


def save():
    path = ROOT / 'owned.json'
    temp = ROOT / 'owned.new'
    if temp.exists():
        temp.unlink()
    private(temp, json.dumps(STATE))
    temp.replace(path)


def tart(*args, **kwargs):
    return run([TART, *args], env=ENV, **kwargs)


def guest(script, **kwargs):
    return tart('exec', '-i', GUEST, '/bin/bash', '-s', data=script.encode(), **kwargs)


def spawn(args, log, *, cwd=REPO, env=None):
    handle = open(log, 'xb', opener=lambda path, flags: os.open(path, flags, 0o600))
    executable = Path(shutil.which(str(args[0])) or str(args[0])).resolve(strict=True)
    process = subprocess.Popen([str(executable), *[str(a) for a in args[1:]]], cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=handle, stderr=subprocess.STDOUT, start_new_session=True)
    handle.close()
    CHILDREN.append(process)
    fingerprint = process_fingerprint(process.pid)
    if fingerprint:
        STATE.setdefault('processes', []).append({'pid':process.pid, 'fingerprint':fingerprint})
        save()
    return process


def process_fingerprint(pid):
    return run(['ps','-p',str(pid),'-o','lstart=','-o','command='],check=False).stdout.decode().strip()


def stop_processes():
    remaining = []
    for record in reversed(STATE.get('processes', [])):
        pid = record['pid']
        if process_fingerprint(pid) != record['fingerprint']:
            continue
        try:
            if os.getpgid(pid) != pid:
                remaining.append(record)
                continue
            os.killpg(pid, signal.SIGTERM)
            deadline = time.monotonic() + 5
            while process_fingerprint(pid) == record['fingerprint'] and time.monotonic() < deadline:
                time.sleep(0.1)
            if process_fingerprint(pid) == record['fingerprint'] and os.getpgid(pid) == pid:
                os.killpg(pid, signal.SIGKILL)
                deadline = time.monotonic() + 3
                while process_fingerprint(pid) == record['fingerprint'] and time.monotonic() < deadline:
                    time.sleep(0.1)
                if process_fingerprint(pid) == record['fingerprint']:
                    remaining.append(record)
        except ProcessLookupError:
            pass
    if remaining:
        STATE['processes'] = remaining
    else:
        STATE.pop('processes', None)
    save()
    if remaining:
        raise RuntimeError('owned process cleanup incomplete; ownership retained for vm-stop recovery')


def wait_for(predicate, label, seconds=120):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(1)
    raise RuntimeError(f'timed out: {label}')


def port_open(port):
    with socket.socket() as sock:
        sock.settimeout(0.2)
        return sock.connect_ex(('127.0.0.1', port)) == 0


def require_ports():
    for port in range(8788, 8792):
        for kind in (socket.SOCK_STREAM, socket.SOCK_DGRAM):
            with socket.socket(type=kind) as sock:
                try:
                    sock.bind(('127.0.0.1', port))
                except OSError as error:
                    raise RuntimeError(f'127.0.0.1:{port} is occupied; stop its owner yourself') from error


def setup():
    if os.uname().machine != 'arm64' or run(['sw_vers', '-productVersion']).stdout.split(b'.')[0] != b'27':
        raise RuntimeError('this harness requires an Apple silicon macOS27 host')
    if not TART.exists():
        tools = ROOT / 'tools'
        tools.mkdir(mode=0o700, exist_ok=True)
        archive = tools / 'tart.tar.gz'
        with urllib.request.urlopen(f'https://github.com/openai/tart/releases/download/{TART_VERSION}/tart.tar.gz') as source:
            private(archive, source.read())
        if hashlib.sha256(archive.read_bytes()).hexdigest() != TART_SHA:
            raise RuntimeError('Tart archive checksum mismatch')
        run(['tar', '-xzf', archive, '-C', tools])
    if tart('--version').stdout.decode().strip() != TART_VERSION:
        raise RuntimeError('unexpected Tart version')
    run(['codesign', '--verify', '--deep', '--strict', TART.parents[2]])
    run(['spctl', '--assess', '--type', 'execute', TART.parents[2]])
    names = json.loads(tart('list', '--format', 'json').stdout)
    present = {item['Name'] for item in names}
    if BASE not in present:
        if shutil.disk_usage(ROOT).free < 80 * 1024**3:
            raise RuntimeError('reserve 80 GiB free before the 34 GB image download')
        print('Downloading pinned macOS27 base (34.2GB); allow 80 GiB free', flush=True)
        tart('clone', IMAGE, BASE, timeout=3600)
        STATE['baseline'] = IMAGE
        save()
    elif STATE.get('baseline') != IMAGE:
        raise RuntimeError('existing baseline is not recorded as this pinned image')
    if not STATE.get('simulator'):
        runtimes = json.loads(run(['xcrun', 'simctl', 'list', 'runtimes', '-j']).stdout)['runtimes']
        runtime = next((r for r in reversed(runtimes) if r['isAvailable'] and r['version'].split('.')[0] == '27' and '.iOS-' in r['identifier']), None)
        if not runtime:
            raise RuntimeError('install an iOS27 Simulator runtime in Xcode first')
        identifier = run(['xcrun', 'simctl', 'create', 'Collie VM test', 'com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro', runtime['identifier']]).stdout.decode().strip()
        STATE['simulator'] = identifier
        STATE['runtime'] = runtime['identifier']
        save()
    if STATE.get('simulator') and not owned_simulator():
        raise RuntimeError('recorded dedicated Simulator is missing; run vm-clean then vm-setup')
    if not STATE.get('desktop'):
        prepare_desktop()
    print(f'Setup ready: pinned macOS27 baseline; dedicated Simulator {STATE["simulator"]}', flush=True)


def owned_simulator():
    identifier = STATE.get('simulator')
    if not identifier:
        return None
    devices = json.loads(run(['xcrun', 'simctl', 'list', 'devices', '-j']).stdout)['devices']
    device = next((item for group in devices.values() for item in group if item['udid'] == identifier), None)
    if device and device['name'] != 'Collie VM test':
        raise RuntimeError('recorded Simulator has an unexpected name; refusing to change it')
    return identifier if device else None


def stop():
    stop_processes()
    if STATE.get('guest'):
        tart('stop', GUEST, check=False)
    simulator = owned_simulator()
    if simulator:
        run(['xcrun', 'simctl', 'shutdown', simulator], check=False)


def reset():
    stop()
    if STATE.get('guest'):
        names = {item['Name'] for item in json.loads(tart('list', '--format', 'json').stdout)}
        if GUEST in names:
            tart('delete', GUEST)
        STATE.pop('guest')
        save()
    simulator = owned_simulator()
    if simulator:
        run(['xcrun', 'simctl', 'erase', simulator])
    print('Owned guest deleted and dedicated Simulator erased; baseline retained', flush=True)


def boot(log, graphical=False):
    inventory = {item['Name']: item for item in json.loads(tart('list', '--format', 'json').stdout)}
    names = set(inventory)
    if GUEST in names and not STATE.get('guest'):
        raise RuntimeError('unrecorded guest exists; refusing to adopt or replace it')
    if GUEST not in names:
        tart('clone', READY if STATE.get('desktop') else BASE, GUEST)
        STATE['guest'] = True
        save()
    running = inventory.get(GUEST, {}).get('Running', False)
    if not running:
        tart('set', GUEST, '--cpu', '2', '--memory', '4096')
        args = [TART, 'run', GUEST, '--no-audio', '--no-usb-accessories', '--no-clipboard', '--net-host']
        if not graphical:
            args.append('--no-graphics')
        with open(log, 'xb', opener=lambda path, flags: os.open(path, flags, 0o600)) as output:
            subprocess.Popen([str(a) for a in args], env=ENV, stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
    wait_for(port_guest_ready, 'macOS27 guest agent', 240)
    version = guest('sw_vers -productVersion').stdout.decode().strip()
    if version.split('.')[0] != '27':
        raise RuntimeError(f'guest must be macOS 27, found {version}')
    return guest('sw_vers; /usr/sbin/spctl --status; /usr/bin/csrutil status').stdout


def port_guest_ready():
    try:
        return tart('exec', GUEST, '/usr/bin/true', timeout=3, check=False).returncode == 0
    except subprocess.TimeoutExpired:
        return False


def prepare_desktop():
    boot(ROOT / f'desktop-boot-{time.monotonic_ns()}.log', graphical=True)
    archive = ROOT / 'tools/cua-driver-0.36.0.tar.gz'
    if not archive.exists():
        with urllib.request.urlopen(f'https://github.com/trycua/cua/releases/download/cua-driver-rs-v{DRIVER_VERSION}/cua-driver-rs-{DRIVER_VERSION}-darwin-arm64.tar.gz') as source:
            private(archive, source.read())
    if hashlib.sha256(archive.read_bytes()).hexdigest() != DRIVER_SHA:
        raise RuntimeError('Cua Driver archive checksum mismatch')
    current = tart('exec', GUEST, DRIVER, '--version', check=False).stdout.decode().strip()
    if current != f'cua-driver {DRIVER_VERSION}':
        tart('exec', '-i', GUEST, '/bin/bash', '-c', 'umask 077; mkdir -p /private/tmp/collie-cua-eval; cat > /private/tmp/collie-cua-eval/cua.tar.gz', data=archive.read_bytes())
        guest(f'set -e; cd /private/tmp/collie-cua-eval; echo "{DRIVER_SHA}  cua.tar.gz" | shasum -a 256 -c -; tar -xzf cua.tar.gz; test ! -e /Applications/CuaDriver.app; cp -R cua-driver-rs-{DRIVER_VERSION}-darwin-arm64/CuaDriver.app /Applications/CuaDriver.app')
    guest(f'codesign --verify --deep --strict /Applications/CuaDriver.app; {DRIVER} telemetry disable; open -n -g -a /Applications/CuaDriver.app --args serve')
    print('Inside the collie-374-test VM, grant CuaDriver Accessibility and Screen & System Audio Recording; accept relaunch. Then run just vm-desktop-ready.', flush=True)


def driver_call(tool, arguments=None):
    arguments = {'session':'collie-vm-proof', **(arguments or {})}
    return json.loads(tart('exec', GUEST, DRIVER, 'call', tool, json.dumps(arguments)).stdout)


def desktop_proof(proof):
    guest('umask 077; mkdir -p /private/tmp/collie-cua-eval')
    driver_call('launch_app', {'bundle_id':'com.apple.calculator'})
    window = None
    for _ in range(15):
        windows = driver_call('list_windows', {'app':'Calculator'})['windows']
        if windows:
            window = windows[0]
            break
        time.sleep(1)
    if not window:
        raise RuntimeError('guest Calculator did not expose a controllable window')
    target = {'pid':window['pid'], 'window_id':window['window_id']}
    state = driver_call('get_window_state', {**target, 'include_screenshot':False})
    clear = re.search(r'\[(\d+)\] AXButton \((?:All Clear|Clear)\)', state['tree_markdown'])
    if not clear:
        raise RuntimeError('guest Calculator clear button missing')
    driver_call('click', {'element_token':state['snapshot_id']+':'+clear[1]})
    before = driver_call('get_window_state', {**target, 'screenshot_out_file':'/private/tmp/collie-cua-eval/calculator-before.png'})
    six = re.search(r'\[(\d+)\] AXButton \(6\)', before['tree_markdown'])
    if not six or 'AXStaticText = "0"' not in before['tree_markdown']:
        raise RuntimeError('guest Calculator initial display is not zero')
    driver_call('click', {'element_token':before['snapshot_id']+':'+six[1]})
    after = driver_call('get_window_state', {**target, 'since':'latest', 'screenshot_out_file':'/private/tmp/collie-cua-eval/calculator-after.png'})
    if 'AXStaticText = "6"' not in after.get('tree_diff', after.get('tree_markdown','')):
        raise RuntimeError('guest Calculator did not visibly change from zero to six')
    proof.mkdir(exist_ok=True, mode=0o700)
    for name, value in [('calculator-before',before),('calculator-after',after)]:
        for suffix, content in [('.json',json.dumps(value).encode()),('.png',tart('exec',GUEST,'/bin/cat',f'/private/tmp/collie-cua-eval/{name}.png').stdout)]:
            destination = proof / (name+suffix)
            if destination.exists(): destination.unlink()
            private(destination,content)


def desktop_ready():
    if not STATE.get('guest') or not port_guest_ready():
        raise RuntimeError('run just vm-setup and complete guest-only CuaDriver consent first')
    permission = json.loads(tart('exec', GUEST, DRIVER, 'permissions', 'status', '--json').stdout)
    if permission.get('accessibility') is not True or permission.get('screen_recording') is not True or permission.get('source', {}).get('attribution') != 'driver-daemon':
        raise RuntimeError('guest Driver has not verified both permissions as driver-daemon; complete the visible guest consent, then retry')
    proof = ROOT / 'desktop-proof'
    desktop_proof(proof)
    apps = tart('exec', GUEST, DRIVER, 'call', 'list_apps').stdout
    state = tart('exec', GUEST, DRIVER, 'call', 'get_desktop_state', json.dumps({'session':'collie-vm-setup','screenshot_out_file':'/private/tmp/collie-cua-eval/desktop.png'})).stdout
    if b'permission' in apps.lower() and b'pending' in apps.lower() or b'isError":true' in apps or b'isError":true' in state:
        raise RuntimeError('complete the guest System Events/direct screen capture prompts and retry vm-desktop-ready')
    image = tart('exec', GUEST, '/bin/cat', '/private/tmp/collie-cua-eval/desktop.png').stdout
    if not image.startswith(b'\x89PNG\r\n\x1a\n'):
        raise RuntimeError('Driver did not produce a guest desktop screenshot')
    proof.mkdir(exist_ok=True, mode=0o700)
    for name, content in [('permissions.json',json.dumps(permission).encode()),('apps.json',apps),('state.json',state),('desktop.png',image)]:
        destination = proof / name
        if destination.exists(): destination.unlink()
        private(destination,content)
    guest(f'set -e; test ! -e "/Users/admin/Library/Application Support/collie-vm-test"; test ! -e /private/tmp/collie-vm-test; test ! -e /Users/admin/.collie-vm-test; {DRIVER} stop; rm -f /private/tmp/collie-cua-eval/cua.tar.gz')
    tart('stop',GUEST)
    if STATE.get('desktop'):
        raise RuntimeError('prepared desktop baseline already exists; do not replace it implicitly')
    tart('clone',GUEST,READY)
    STATE['desktop'] = DRIVER_VERSION
    save()
    print(f'Guest desktop grants and capture verified; prepared baseline {READY}. Private proof: {proof}',flush=True)


def wrong_core_proof(logdir, environment):
    library = REPO / 'target/aarch64-apple-ios-sim/release/libcollie_core.a'
    if not library.is_file() or b'collie_vm_test_supported' in library.read_bytes():
        raise RuntimeError('wrong-core negative control must use an ordinary Release core')
    command = ['xcodebuild','-project','Collie/Collie.xcodeproj','-scheme','Collie','-derivedDataPath','target/ios/VMWrongCore','-destination',f'platform=iOS Simulator,id={STATE["simulator"]}','CODE_SIGNING_ALLOWED=YES','CODE_SIGN_IDENTITY=-','CODE_SIGN_STYLE=Automatic','build']
    result = run(command,env=environment,timeout=1800)
    private(logdir/'build-wrong-core.log',result.stdout+result.stderr)
    app = REPO/'target/ios/VMWrongCore/Build/Products/Debug-iphonesimulator/Collie.app'
    signature = run(['codesign','-dvv',app]).stderr
    if b'Signature=adhoc' not in signature:
        raise RuntimeError('Simulator test app must use ad-hoc signing, without a personal certificate')
    private(logdir/'wrong-core-signature.log',signature)
    executable = app/'Collie.debug.dylib'
    if b'collie_vm_test_supported' in run(['nm','-gU',executable]).stdout:
        raise RuntimeError('wrong-core executable unexpectedly exports the VM capability')
    run(['xcrun','simctl','install',STATE['simulator'],app])
    container = Path(run(['xcrun','simctl','get_app_container',STATE['simulator'],'dev.rbstp.collie','data']).stdout.decode().strip())
    state_dirs = [container/'Library/Application Support'/name for name in ('collie','collie-vm-test')]
    if any(path.exists() for path in state_dirs):
        raise RuntimeError('wrong-core negative control requires a fresh dedicated app container')
    launch = run(['xcrun','simctl','launch',STATE['simulator'],'dev.rbstp.collie'],env={**environment,'SIMCTL_CHILD_COLLIE_VM_TEST':'1'})
    time.sleep(3)
    run(['xcrun','simctl','io',STATE['simulator'],'screenshot',logdir/'wrong-core.png'])
    if any(path.exists() for path in state_dirs):
        raise RuntimeError('wrong-profile core created state before refusing VM mode')
    identity = json.loads((container/'tmp/vm-core-rejection.json').read_text())
    private(logdir/'wrong-core-identity.json',json.dumps(identity))
    if len(identity) != 4 or any(item['status'] != -25300 or item['count'] != 0 for item in identity):
        raise RuntimeError('wrong-core mode accessed an existing TLS/terminal identity or could not prove its absence')
    private(logdir/'wrong-core.json',json.dumps({'core':'ordinary Release','swift':'Debug Simulator','requested_vm_mode':True,'normal_and_vm_state_absent':True,'launch':launch.stdout.decode(),'screenshot':'wrong-core.png'}))
    run(['xcrun','simctl','terminate',STATE['simulator'],'dev.rbstp.collie'])
    run(['xcrun','simctl','uninstall',STATE['simulator'],'dev.rbstp.collie'])


def build(logdir):
    environment = {**os.environ, 'GO': '/opt/homebrew/opt/go/bin/go'}
    commands = [(['cargo', 'build', '-p', 'collied'], REPO),
                ([environment['GO'], 'build', '-o', str(logdir / 'testcontrol'), '.'], REPO / 'crates/tailnet/testcontrol'),
                (['just', 'ios-framework', 'sim', 'release'], REPO),
                (['just', 'ios-project'], REPO),
                (['npm', 'ci'], REPO / 'relay')]
    for index, (command, cwd) in enumerate(commands):
        print('Building: ' + ' '.join(command[:3]), flush=True)
        result = run(command, cwd=cwd, env=environment, timeout=1800)
        private(logdir / f'build-{index}.log', result.stdout + result.stderr)
    run(['xcrun', 'simctl', 'boot', STATE['simulator']], check=False)
    run(['xcrun', 'simctl', 'bootstatus', STATE['simulator'], '-b'], timeout=180)
    wrong_core_proof(logdir,environment)
    result = run(['just','ios-framework','sim','debug'],env=environment,timeout=1800)
    private(logdir/'build-debug-core.log',result.stdout+result.stderr)
    command = ['xcodebuild', '-project', 'Collie/Collie.xcodeproj', '-scheme', 'Collie', '-derivedDataPath', 'target/ios/VMTest', '-destination', f'platform=iOS Simulator,id={STATE["simulator"]}', 'CODE_SIGNING_ALLOWED=YES', 'CODE_SIGN_IDENTITY=-', 'CODE_SIGN_STYLE=Automatic', 'COLLIE_VM_TEST_LDFLAGS=-Wl,-u,_collie_vm_test_supported -Wl,-export_dynamic', 'build']
    result = run(command, env=environment, timeout=1800)
    private(logdir / 'build-ios.log', result.stdout + result.stderr)
    app = REPO / 'target/ios/VMTest/Build/Products/Debug-iphonesimulator/Collie.app'
    symbols = run(['nm', '-gU', app / 'Collie.debug.dylib']).stdout
    if b'_collie_vm_test_supported' not in symbols:
        raise RuntimeError('Debug Simulator executable lacks the required test capability')
    return app


def exercise():
    setup()
    if not STATE.get('desktop'):
        raise RuntimeError('finish one-time guest desktop consent and vm-desktop-ready before vm-test')
    if STATE.get('processes'):
        raise RuntimeError('recorded test processes remain; run vm-stop then vm-reset before testing')
    require_ports()
    if STATE.get('last_run'):
        raise RuntimeError('run vm-reset before another test to ensure fresh identities and state')
    run_id = str(uuid.uuid4())
    logdir = ROOT / 'runs' / run_id
    logdir.mkdir(parents=True, mode=0o700)
    STATE['last_run'] = run_id
    save()
    private(logdir / 'guest-posture.txt', boot(logdir / 'vm.log'))
    guest(f'open -n -g -a /Applications/CuaDriver.app --args serve')
    permission = json.loads(tart('exec', GUEST, DRIVER, 'permissions', 'status', '--json').stdout)
    if permission.get('accessibility') is not True or permission.get('screen_recording') is not True or permission.get('source', {}).get('attribution') != 'driver-daemon':
        raise RuntimeError('prepared guest lost its Driver grants; rerun vm-desktop-ready setup rather than bypassing consent')
    private(logdir / 'driver-permissions.json', json.dumps(permission))
    desktop_proof(logdir / 'desktop')
    app = build(logdir)
    marker_dir = logdir / 'collie-vm-test'
    marker_dir.mkdir(mode=0o700)
    marker = json.dumps({'auth_key': 'test-authkey-collievm' + uuid.uuid4().hex, 'run_id': run_id})
    private(marker_dir / 'vm-tailnet.json', marker)
    relay = logdir / 'relay'
    relay_env = {key: value for key, value in os.environ.items() if key in ('PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL')}
    run(['node', 'dev/simulator.ts', 'init', relay], cwd=REPO / 'relay', env=relay_env)
    spawn([logdir / 'testcontrol', '-vm-dir', marker_dir, '-same-user'], logdir / 'control.log')
    spawn(['node', 'dev/simulator.ts', 'serve', relay], logdir / 'relay.log', cwd=REPO / 'relay', env=relay_env)
    wait_for(lambda: port_open(8788) and port_open(8789) and port_open(8790), 'local control/DERP/mock relay')
    key = logdir / 'ssh-key'
    run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', key])
    guest_root = '/Users/admin/.collie-vm-test'
    guest_data = '/Users/admin/Library/Application Support/collie-vm-test'
    files = {'collied': (REPO / 'target/debug/collied').read_bytes(), 'testcontrol': (logdir / 'testcontrol').read_bytes(),
             'authorized_keys': key.with_suffix('.pub').read_bytes(), 'vm-tailnet.json': marker.encode(), 'collied.toml': b'[tailnet]\nhostname = "collie-vm-test"\n[push.relay]\n'}
    script = f'set -euo pipefail\numask077\nmkdir -p {shlex.quote(guest_root)} {shlex.quote(guest_data)}\nchmod700 {shlex.quote(guest_root)} {shlex.quote(guest_data)}\n'
    script = script.replace('umask077', 'umask 077').replace('chmod700', 'chmod 700')
    for name, content in files.items():
        directory = guest_data if name in ('vm-tailnet.json', 'collied.toml') else guest_root
        script += f"/usr/bin/base64 -D > {shlex.quote(directory + '/' + name)} <<'COLLIE_FILE'\n{base64.b64encode(content).decode()}\nCOLLIE_FILE\n"
    script += f'chmod700 {guest_root}/collied {guest_root}/testcontrol\n'.replace('chmod700', 'chmod 700')
    script += f'ssh-keygen -q -t ed25519 -N "" -f {guest_root}/host-key\n'
    configuration = f"""Port 2222
ListenAddress 0.0.0.0
HostKey {guest_root}/host-key
PidFile {guest_root}/sshd.pid
AuthorizedKeysFile {guest_root}/authorized_keys
AllowUsers admin
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes yes
GatewayPorts no
AllowAgentForwarding no
AllowTcpForwarding remote
PermitListen 127.0.0.1:8788 127.0.0.1:8789 127.0.0.1:8790
PermitTunnel no
X11Forwarding no
"""
    script += f"cat > {guest_root}/sshd_config <<'COLLIE_SSHD'\n{configuration}COLLIE_SSHD\nsudo /usr/sbin/sshd -t -f {guest_root}/sshd_config\nsudo /usr/sbin/sshd -E {guest_root}/sshd.log -f {guest_root}/sshd_config\ncat {guest_root}/host-key.pub\n"
    hostkey = guest(script, timeout=120).stdout.decode().strip()
    address = tart('ip', GUEST, '--wait', '60').stdout.decode().strip()
    if not re.fullmatch(r'192\.168\.\d{1,3}\.\d{1,3}', address) or not hostkey.startswith('ssh-ed25519 '):
        raise RuntimeError('unexpected isolated guest IP or host key')
    known = logdir / 'known_hosts'
    private(known, f'[{address}]:2222 {hostkey}\n')
    proxy = shlex.join(['/usr/bin/env', f'TART_HOME={ROOT / "tart"}', 'TART_NO_AUTO_PRUNE=1', str(TART), 'exec', '-i', GUEST, '/usr/bin/nc', '127.0.0.1', '2222'])
    ssh = ['/usr/bin/ssh', '-F', '/dev/null', '-i', str(key), '-p', '2222', '-o', 'IdentitiesOnly=yes', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5', '-o', 'StrictHostKeyChecking=yes', '-o', f'UserKnownHostsFile={known}', '-o', 'ForwardAgent=no', '-o', 'ExitOnForwardFailure=yes', '-o', 'GatewayPorts=no', '-o', f'ProxyCommand={proxy}']
    remote = [*ssh, f'admin@{address}']
    def ssh_ready():
        result = run([*remote, 'true'], check=False, timeout=10)
        private(logdir / f'ssh-ready-{time.monotonic_ns()}.log', result.stdout + result.stderr)
        return result.returncode == 0
    wait_for(ssh_ready, 'strict guest SSH authentication', 60)
    forwards = [value for port in (8788,8789,8790) for value in ('-R', f'127.0.0.1:{port}:127.0.0.1:{port}')]
    spawn([*ssh, *forwards, '-N', f'admin@{address}'], logdir / 'ssh.log')
    time.sleep(2)
    spawn([*remote, f'{guest_root}/testcontrol -vm-dir {shlex.quote(guest_data)} -stun-only'], logdir / 'guest-stun.log')
    prefix = f'COLLIE_VM_TEST=1 COLLIE_SIMULATOR_RELAY=1 {guest_root}/collied'
    result = run([*remote, prefix + ' login'], timeout=120)
    private(logdir / 'guest-login.log', result.stdout + result.stderr)
    spawn([*remote, prefix + ' run'], logdir / 'daemon.log')
    wait_for(lambda: run([*remote, prefix + ' status'], check=False).returncode == 0, 'guest daemon')
    run(['xcrun', 'simctl', 'install', STATE['simulator'], app])
    container = Path(run(['xcrun', 'simctl', 'get_app_container', STATE['simulator'], 'dev.rbstp.collie', 'data']).stdout.decode().strip())
    sim_data = container / 'Library/Application Support/collie-vm-test'
    sim_data.mkdir(parents=True, mode=0o700)
    private(sim_data / 'vm-tailnet.json', marker)
    private(sim_data / 'simulator-relay.json', (relay / 'simulator-relay.json').read_bytes())
    pair = subprocess.Popen([*remote, prefix + ' pair --show-uri'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    CHILDREN.append(pair)
    selector = selectors.DefaultSelector()
    selector.register(pair.stdout, selectors.EVENT_READ)
    output = b''
    launched = confirmed = False
    deadline = time.monotonic() + 150
    while pair.poll() is None and time.monotonic() < deadline:
        for event, _ in selector.select(1):
            output += os.read(event.fileobj.fileno(), 65536)
        invite = re.search(rb'collie://pair#[^\s]+', output)
        if invite and not launched:
            private(sim_data / 'vm-pairing.txt', invite.group())
            environment = {**os.environ, 'SIMCTL_CHILD_COLLIE_VM_TEST':'1', 'SIMCTL_CHILD_COLLIE_SIMULATOR_RELAY':'mock'}
            run(['xcrun', 'simctl', 'launch', STATE['simulator'], 'dev.rbstp.collie'], env=environment)
            launched = True
        if b'Pair this phone? [y/N]' in output and not confirmed:
            if b'device:    VM Simulator' not in output:
                raise RuntimeError('unexpected pairing candidate; refusing confirmation')
            pair.stdin.write(b'y\n')
            pair.stdin.flush()
            confirmed = True
    if pair.poll() is None or pair.returncode != 0 or not confirmed:
        raise RuntimeError('normal mutual-TLS pairing failed; inspect the dedicated Simulator and private daemon log')
    private(logdir / 'pairing.log', 'Confirmed only VM Simulator through the ordinary mutual-TLS pairing prompt.\n')
    wait_for(lambda: not (sim_data / 'vm-pairing.txt').exists(), 'Simulator paired state')
    identity = json.loads((container/'tmp/vm-core-identity.json').read_text())
    private(logdir/'paired-identity.json',json.dumps(identity))
    if len(identity) != 4 or any(item['status'] != -25300 or item['count'] != 0 for item in identity[:2]) or any(item['status'] != 0 or item['count'] != 1 for item in identity[2:]):
        raise RuntimeError('paired Simulator identity was not confined to the VM test namespace')
    def pushed():
        result = run([*remote, prefix + ' apns test'], check=False)
        private(logdir / f'push-{time.monotonic_ns()}.log', result.stdout + result.stderr)
        return result.returncode == 0 and (relay / 'last-push.apns').is_file()
    wait_for(pushed, 'encrypted mock relay push', 90)
    payload = relay / 'last-push.apns'
    delivered = payload.read_bytes()
    if 'enc' not in json.loads(delivered):
        raise RuntimeError('mock delivery did not contain the encrypted display envelope')
    revoked = run([*remote, prefix + " peers revoke 'VM Simulator'"])
    private(logdir / 'revocation.log', revoked.stdout + revoked.stderr)
    rejected = run([*remote, prefix + ' apns test'],check=False)
    private(logdir / 'revoked-push.log', rejected.stdout + rejected.stderr)
    if rejected.returncode == 0 or b'no registered device' not in rejected.stdout or payload.read_bytes() != delivered:
        raise RuntimeError('revoked peer still accepted a mock push or changed delivery evidence')
    identities = [{**record, 'live':process_fingerprint(record['pid'])} for record in STATE.get('processes',[])]
    private(logdir/'process-identities.json',json.dumps(identities))
    if not identities or any(record['fingerprint'] != record['live'] for record in identities):
        raise RuntimeError('owned integration process changed identity before cleanup; inspect private evidence')
    private(logdir / 'result.json', json.dumps({'result':'PASS', 'revision':run(['git','rev-parse','HEAD']).stdout.decode().strip(), 'dirty':bool(run(['git','status','--porcelain']).stdout), 'run_id':run_id, 'simulator':STATE['simulator'], 'guest':'macOS27', 'pairing':'ordinary confirmed mutual TLS', 'push':'encrypted local mock only', 'revocation':'ordinary peer revoke; follow-up rejected without new delivery', 'genuine_app_attest':False, 'real_apns':False}))
    print(f'PASS: guest/Simulator pairing and encrypted mock push; private artifacts: {logdir}', flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=['setup','desktop-ready','test','reset','stop','clean'])
    args = parser.parse_args()
    os.umask(0o077)
    ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
    if ROOT.is_symlink() or ROOT.stat().st_uid != os.getuid() or ROOT.stat().st_mode & 0o077:
        raise RuntimeError('task root must be private and owned, without symlinks')
    with open(ROOT / 'lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        global STATE
        if (ROOT / 'owned.json').exists():
            STATE = json.loads((ROOT / 'owned.json').read_text())
            if STATE.get('owner') != 'collie-vm-simulator-v1':
                raise RuntimeError('unrecognized ownership manifest')
        elif any(path != ROOT / 'lock' for path in ROOT.iterdir()):
            raise RuntimeError('existing task directory has no ownership manifest')
        else:
            STATE = {'owner':'collie-vm-simulator-v1'}
            save()
        try:
            if args.command == 'setup': setup()
            elif args.command == 'desktop-ready': desktop_ready()
            elif args.command == 'test': exercise()
            elif args.command == 'stop': stop()
            elif args.command == 'reset':
                reset()
                STATE.pop('last_run', None)
                save()
            else:
                reset()
                simulator = owned_simulator()
                if simulator:
                    run(['xcrun','simctl','delete',simulator])
                if STATE.get('desktop'):
                    tart('delete',READY)
                if STATE.get('baseline'):
                    tart('delete',BASE)
                    tart('delete',IMAGE,check=False)
                STATE.clear()
                STATE['owner'] = 'collie-vm-simulator-v1'
                save()
                print(f'Owned VMs and Simulator removed. Private logs/tools remain at {ROOT}; remove that exact directory if no longer needed.')
        finally:
            if args.command == 'test':
                stop()
            for process in reversed(CHILDREN):
                if process.poll() is None: process.terminate()
            for process in reversed(CHILDREN):
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired: process.kill(); process.wait()
                for pipe in (process.stdin, process.stdout, process.stderr):
                    if pipe: pipe.close()


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        print('Stopped owned test resources', file=sys.stderr)
        sys.exit(130)
    except (RuntimeError, subprocess.TimeoutExpired, BlockingIOError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
