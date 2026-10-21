"""Exercise an extracted makepkg payload, never a source fallback or user session.

The parent builds/extracts the package and runs this witness. All Core processes,
PTYs, plugin state, custom declarations and shell receipts belong to temporary
homes. No package bytes are edited and no build/install command is run here.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shlex
import shutil
import struct
import subprocess
import tempfile
import termios
import threading
import tomllib
import time

from command_compatibility import digest, request
from command_definitions import PALETTE_ARTIFACTS, PALETTE_OWNER, until
from command_interactions import exports, native_screen, projection


IDS = {name: 'tommymorgan.' + name for name in
       ('stock-ui', 'sidebar-menus', 'command-palette', 'agent-tree')}


class Session:
    def __init__(self, args, name):
        self.args = args
        self.temporary = tempfile.TemporaryDirectory(prefix='packaged-palette-')
        self.home = Path(self.temporary.name)
        self.evidence = args.output_dir / name
        self.evidence.mkdir(parents=True)
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(('HERDR_', 'TRAILBOSS_'))}
        for key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_DATA_HOME',
                    'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
            self.env[key] = str(self.home)
        self.env.update(PATH='', SHELL='/bin/sh', TERM='xterm-256color',
                        TRAILBOSS_CONFIG_PATH=str(self.home / 'config.toml'),
                        TRAILBOSS_STARTUP_CWD=str(self.home))
        self.env.pop('NO_COLOR', None)
        assert shutil.which('bun', path=self.env['PATH']) is None
        (self.home / 'config.toml').write_text('[keys]\nprefix = "ctrl+a"\n')
        self.api = self.home / 'trailboss/sessions/package/trailboss.sock'
        self.endpoint = self.api.with_name('trailboss-client.sock')
        self.output = bytearray()
        self.master = self.native = self.drain = self.core = None
        self.stop = threading.Event()
        self.stderr = (self.evidence / 'core.stderr').open('wb')

    def save(self, name, value):
        (self.evidence / name).write_text(json.dumps(value, indent=2))

    def call(self, method, params=None):
        result = request(self.api, method, params)
        with (self.evidence / 'api.jsonl').open('a') as log:
            log.write(json.dumps({'method': method, 'params': params or {}, 'result': result}) + '\n')
        return result

    def start(self):
        self.core = subprocess.Popen([str(self.args.client), 'server', '--session', 'package'],
                                     env=self.env, cwd=self.home, stdin=subprocess.DEVNULL,
                                     stdout=subprocess.DEVNULL, stderr=self.stderr)
        def ready():
            assert self.core.poll() is None, 'owned packaged Core exited'
            try:
                return self.endpoint.exists() and self.call('session.snapshot')
            except (OSError, AssertionError):
                return None
        until(ready, 'owned packaged Core startup')
        for workspace in self.call('workspace.list')['workspaces']:
            self.call('workspace.close', {'workspace_id': workspace['workspace_id']})

    def link(self, directory, enabled=True):
        return self.call('plugin.link', {'path': str(self.args.producer_root / 'plugins' / directory),
                                         'enabled': enabled})

    def state(self):
        # Observer hello updates attach bookkeeping. Compare resources and actual
        # focus, not snapshot revision/client counters, to claim passive behavior.
        snapshot = self.call('session.snapshot')['snapshot']
        return {'workspaces': sorted((item['workspace_id'], item['label']) for item in
                                     self.call('workspace.list')['workspaces']),
                'tabs': sorted((item['tab_id'], item['workspace_id'], item['label']) for item in
                               self.call('tab.list')['tabs']),
                'panes': sorted(item['pane_id'] for item in self.call('pane.list')['panes']),
                'focus': {key: snapshot.get(key) for key in
                          ('focused_workspace_id', 'focused_tab_id', 'focused_pane_id')}}

    def published(self):
        def available():
            value = projection(self.endpoint)
            palettes = [(owner, fragment) for owner, fragment in exports(value)
                        if fragment['name'] == 'command-palette']
            if not palettes:
                return None
            assert len(palettes) == 1 and palettes[0][0] == PALETTE_OWNER, (
                'competing bundled palette providers', palettes)
            return value
        return until(available, 'compiled dedicated producer publishes the sole bundled palette')

    def attach(self):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 110, 0, 0))
        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        try:
            self.native = subprocess.Popen([str(self.args.client), '--session', 'package'],
                                           env=self.env, cwd=self.home, stdin=slave, stdout=slave,
                                           stderr=slave, preexec_fn=controlling_terminal,
                                           close_fds=True)
        finally:
            os.close(slave)
        def pump():
            while not self.stop.is_set():
                try:
                    readable, _, _ = select.select([self.master], [], [], 0.05)
                    if readable:
                        block = os.read(self.master, 65536)
                        if not block:
                            return
                        self.output.extend(block)
                except OSError:
                    return
        self.drain = threading.Thread(target=pump, daemon=True)
        self.drain.start()

    def keys(self, data):
        assert self.native.poll() is None, ('packaged native exited', bytes(self.output))
        os.write(self.master, data)
        time.sleep(0.08)  # Separate native input events, not a bracketed paste.

    def screen(self):
        assert self.native.poll() is None, ('packaged native exited', bytes(self.output))
        return native_screen(bytes(self.output))

    def visible(self, text):
        return until(lambda: any(text in line for line in self.screen()),
                     'packaged native displays ' + text)

    def absent(self, text):
        return until(lambda: all(text not in line for line in self.screen()),
                     'packaged native closes ' + text)

    def shell_receipt(self, label):
        path = self.home / (label + '.shell')
        command = "{ printf '%s ' \"$HERDR_PANE_ID\"; /usr/bin/stty size; } > " + shlex.quote(str(path))
        self.keys(command.encode() + b'\r')
        until(lambda: path.is_file() and len(path.read_text().split()) == 3,
              'live shell receives native input: ' + label)
        value = path.read_text().split()
        self.save(label + '.json', {'pane_id': value[0], 'rows': int(value[1]), 'cols': int(value[2])})
        return value

    def close(self):
        if self.core and self.core.poll() is None:
            for name, method in (('plugins.json', 'plugin.list'), ('plugin-logs.json', 'plugin.log.list')):
                try:
                    self.save(name, self.call(method))
                except (OSError, AssertionError) as error:
                    self.save(name, {'error': str(error)})
        if self.native and self.native.poll() is None:
            self.native.terminate()
            try:
                self.native.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.native.kill()
                self.native.wait()
        self.stop.set()
        if self.drain:
            self.drain.join(timeout=1)
        if self.master is not None:
            os.close(self.master)
        (self.evidence / 'native.ansi').write_bytes(self.output)
        if self.core and self.core.poll() is None:
            try:
                self.call('server.stop')
                self.core.wait(timeout=5)
            except (OSError, AssertionError, subprocess.TimeoutExpired):
                self.core.terminate()
                self.core.wait(timeout=3)
        self.stderr.close()
        log = self.api.with_name('trailboss-server.log')
        if log.is_file():
            shutil.copy2(log, self.evidence / 'core.log')
        self.temporary.cleanup()


def empty_case(session):
    before = session.state()
    assert not before['workspaces'] and not before['tabs'] and not before['panes'], before
    session.link('command-palette')
    session.save('first-empty-projection.json', session.published())
    initialized = session.state()
    # The existing Core initializes its first workspace on any first endpoint
    # attachment, including an inactive observer. This is not a zero-effect API.
    assert len(initialized['workspaces']) == len(initialized['tabs']) == len(initialized['panes']) == 1
    session.attach()
    session.visible('~')
    session.published()
    assert session.state() == initialized, 'default activation changed the initialized resources/focus'
    session.save('passive-empty.json', {'before': before, 'after_observer': initialized,
                                      'after_native': session.state(),
                                      'existing_first_attachment_initialization': True})


def live_case(session):
    created = session.call('workspace.create', {'label': 'Packaged workspace',
                                               'cwd': str(session.home), 'focus': True})
    pane = created['root_pane']['pane_id']
    session.attach()
    session.visible('Packaged workspace')
    required = {(IDS['sidebar-menus'], name) for name in
                ('sidebar.session-menu', 'commands.confirm-space', 'commands.confirm-tab')}
    def default_ready():
        value = session.published()
        fragments = {(owner, fragment['name']) for owner, fragment in exports(value)}
        stock = any(item['plugin_id'] == IDS['stock-ui'] and
                    item.get('declaration', {}).get('composition') for item in value['ui']
                    if item.get('declaration'))
        return value if required <= fragments and stock else None
    value = until(default_ready, 'stock composition and retained sidebar/confirmation exports publish')
    session.save('default-projection.json', value)
    plugins = session.call('plugin.list')['plugins']
    assert not any(item['plugin_id'] == IDS['agent-tree'] for item in plugins), 'agent tree lost opt-in behavior'
    for directory in ('stock-ui', 'sidebar-menus', 'command-palette'):
        item = next(item for item in plugins if item['plugin_id'] == IDS[directory])
        assert item['enabled'] and Path(item['plugin_root']).resolve() == (
            session.args.producer_root / 'plugins' / directory).resolve(), item
    initial = session.shell_receipt('before')
    assert initial[0] == pane and int(initial[1]) > 0 and int(initial[2]) > 0, (initial, pane)
    session.call('plugin.unlink', {'plugin_id': PALETTE_OWNER})
    before_observer = session.state()
    session.link('command-palette')
    session.published()
    assert session.state() == before_observer, 'dedicated observer changed resources/focus'
    assert session.shell_receipt('after-observer') == initial, 'passive observer resized the live PTY'
    baseline = session.state()
    for shortcut in (b'\x1bOP', b'\x01p'):
        session.keys(shortcut)
        session.visible('Commands')
        session.keys(b'nEwSpAcE')
        session.visible('New space')
        session.keys(b'\x1b')
        session.absent('Commands')
        session.keys(shortcut)
        session.visible('Commands')
        session.keys(b'zzPackageNoSuchCommand')
        session.visible('No matches')
        session.keys(b'\x1b')
        session.absent('Commands')
        assert session.state() == baseline, 'search or Escape changed resources/focus'
    assert session.shell_receipt('after-palette') == initial, 'palette close changed pane identity/geometry'
    session.save('passive-live.json', {'before': before_observer, 'after': session.state(),
                                     'shell_identity_and_geometry': initial,
                                     'observer_bookkeeping_excluded': True})


def enabled_case(session):
    expected = {'stock-ui': True, 'sidebar-menus': False, 'command-palette': False, 'agent-tree': False}
    for directory, enabled in expected.items():
        session.link(directory, enabled)
    session.attach()
    session.visible('~')
    def preserved():
        plugins = {item['plugin_id']: item for item in session.call('plugin.list')['plugins']}
        assert all(plugins[IDS[name]]['enabled'] == enabled for name, enabled in expected.items()), plugins
        # Native startup must reach its event loop, not just leave the preseeded registry unchanged.
        return bool(session.output) and session.native.poll() is None
    until(preserved, 'native startup preserves explicitly configured enabled states')
    value = projection(session.endpoint)
    assert not any(owner == PALETTE_OWNER for owner, _ in exports(value)), value
    session.save('preserved-enabled-states.json', expected)


def custom_case(session):
    custom = session.home / 'custom'
    custom.mkdir()
    tree = {'type': 'column', 'children': [
        {'type': 'row', 'action': 'host:cancel', 'children': [
            {'type': 'label', 'text': 'Custom palette winner', 'role': 'label'}]}]}
    declaration = {'herdr': 'widget_tree', 'version': 1, 'zones': [],
                   'composition': {'priority': 10000, 'tree': {'type': 'column', 'children': [
                       {'type': 'label', 'text': 'Custom layout winner', 'role': 'label'},
                       {'type': 'flex', 'weight': 1, 'child': {'type': 'surface'}}]}},
                   'fragments': [{'name': 'command-palette', 'priority': 10000, 'tree': tree}]}
    payload = custom / 'declaration.json'
    payload.write_text(json.dumps(declaration) + '\n')
    # A public custom declaration is an intentional override, not a replacement
    # for the bundled producer under test. Its absolute shell command needs no Bun.
    command = ['/bin/sh', '-c', '/bin/cat ' + shlex.quote(str(payload)) + '; while read line; do :; done']
    (custom / 'trailboss-plugin.toml').write_text(
        'id = "fixture.package-custom"\nname = "Package custom"\nversion = "1.0.0"\n'
        'min_herdr_version = "0.9.0"\n[[ui]]\nid = "custom"\nsurface = "sidebar"\n'
        'command = ' + json.dumps(command) + '\nstream = true\n')
    session.call('plugin.link', {'path': str(custom), 'enabled': True})
    session.attach()
    session.visible('Custom layout winner')
    def both():
        value = projection(session.endpoint)
        found = [(owner, fragment) for owner, fragment in exports(value)
                 if fragment['name'] == 'command-palette']
        return value if len(found) == 2 and {owner for owner, _ in found} == {
            PALETTE_OWNER, 'fixture.package-custom'} else None
    session.save('custom-and-default-projection.json', until(both, 'custom override and compiled default coexist'))
    session.keys(b'\x1bOP')
    session.visible('Custom palette winner')
    session.keys(b'\x1b')
    session.absent('Custom palette winner')
    session.call('plugin.unlink', {'plugin_id': 'fixture.package-custom'})
    session.absent('Custom layout winner')
    session.published()
    session.keys(b'\x01p')
    session.visible('Commands')
    session.keys(b'\x1b')


def main():
    parser = argparse.ArgumentParser()
    for name in ('client', 'producer-root', 'output-dir'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--no-bun-path', required=True, action='store_true')
    args = parser.parse_args()
    for name in ('client', 'producer_root', 'output_dir'):
        setattr(args, name, getattr(args, name).resolve())
    assert args.client.parent == args.producer_root, 'run the extracted client beside its extracted plugins'
    args.output_dir.mkdir(parents=True, exist_ok=True)
    # Fail on missing payload before any source-tree startup fallback is possible.
    paths = {'client': args.client, **{name: args.producer_root / 'plugins' / name for name in PALETTE_ARTIFACTS}}
    for name, path in paths.items():
        assert path.is_file(), ('extracted package missing required compiled palette payload', name, str(path))
    palette_root = args.producer_root / 'plugins/command-palette'
    manifest = tomllib.loads((palette_root / 'trailboss-plugin.toml').read_text())
    command = manifest['ui'][0]['command']
    executable = (palette_root / command[0]).resolve()
    assert len(command) == 1 and executable == (palette_root / 'command-palette').resolve(), (
        'packaged palette must launch its extracted compiled executable, never source/Bun', command)
    assert os.access(executable, os.X_OK), ('packaged palette executable permission missing', str(executable))
    hashes = {name: digest(path) for name, path in paths.items()}
    receipts = []
    for name, example in (('first-empty', empty_case), ('live-shortcuts', live_case),
                          ('enabled-states', enabled_case), ('custom-priority', custom_case)):
        session = Session(args, name)
        try:
            session.start()
            example(session)
            receipts.append({'case': name, 'passed': True})
        finally:
            session.close()
    assert {name: digest(path) for name, path in paths.items()} == hashes, 'extracted package bytes changed'
    (args.output_dir / 'receipts.json').write_text(json.dumps({
        'cases': receipts, 'artifact_sha256': hashes, 'bun_absent': True,
        'client': str(args.client), 'producer_root': str(args.producer_root)}, indent=2))
    print(json.dumps({'passed_cases': len(receipts), 'receipts': str(args.output_dir / 'receipts.json')}))


if __name__ == '__main__':
    main()
