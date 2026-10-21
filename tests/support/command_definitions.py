"""Supplied compiled Core variations exercised by the actual dedicated palette.

The caller supplies independently verified Core bytes and one current compiled
native/producer bundle. No definition edits, rebuilds, fixture palette, or Bun
fallback occur here. Each case records and rechecks the same consumer hashes.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import struct
import subprocess
import tempfile
import termios
import threading
import time

from command_compatibility import declarations, digest, request


KEY = {'kind': 'core', 'action_id': 'host:focus'}
PRODUCERS = ('agent-tree/agent-tree', 'agent-tree/stock-ui', 'sidebar-menus/sidebar-menus')
PALETTE_PLUGINS = ('agent-tree', 'stock-ui', 'sidebar-menus', 'command-palette')
PALETTE_ARTIFACTS = PRODUCERS + ('command-palette/command-palette',) + tuple(
    f'{plugin}/trailboss-plugin.toml' for plugin in PALETTE_PLUGINS)
PALETTE_OWNER = 'tommymorgan.command-palette'


def until(predicate, message, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        value = predicate()
        if value:
            return value
        assert time.monotonic() < deadline, message
        time.sleep(0.01)


def palette_uses(node, context=None, route=()):
    """Observe opaque public Uses and resource navigation, without inventing rows."""
    context = node.get('context') or context or {}
    if node.get('type') == 'use':
        return [(node['name'], context, route)]
    if node.get('menu'):
        title = ''.join(child.get('text', '') for child in node.get('children', [])
                        if child.get('type') == 'label')
        route += (title,)
    found = []
    for key in ('children', 'entries'):
        for child in node.get(key, []):
            found.extend(palette_uses(child, context, route))
    for key in ('child', 'fallback', 'menu'):
        if node.get(key):
            found.extend(palette_uses(node[key], context, route))
    return found



def run_case(args, label, expected_title):
    fixture = args.fixtures[label]
    binary = Path(fixture['path'])
    assert digest(binary) == fixture['sha256'], (label, 'supplied Core bytes changed')
    evidence = args.output_dir / label
    evidence.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='palette-core-definitions-') as temporary:
        home = Path(temporary)
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(('HERDR_', 'TRAILBOSS_'))}
        for key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_DATA_HOME',
                    'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
            env[key] = str(home)
        env.update(PATH='', SHELL='/bin/sh', TERM='xterm-256color',
                   TRAILBOSS_CONFIG_PATH=str(home / 'config.toml'),
                   TRAILBOSS_STARTUP_CWD=str(home))
        bundle = home / 'bundle'
        bundle.mkdir()
        client_binary = bundle / 'trailboss'
        shutil.copy2(args.client, client_binary)
        plugins = bundle / 'plugins'
        shutil.copytree(args.producer_root / 'plugins', plugins)
        assert digest(client_binary) == args.client_hash
        assert {item: digest(plugins / item) for item in PALETTE_ARTIFACTS} == args.producer_hashes
        session = 'definitions'
        api = home / f'trailboss/sessions/{session}/trailboss.sock'
        endpoint = home / f'trailboss/sessions/{session}/trailboss-client.sock'
        native = None
        master = None
        drain = None
        stop = threading.Event()
        output = bytearray()
        with (evidence / 'core.stderr').open('wb') as stderr:
            core = subprocess.Popen([str(binary), 'server', '--session', session],
                                    cwd=home, env=env, stdin=subprocess.DEVNULL,
                                    stdout=subprocess.DEVNULL, stderr=stderr)
            try:
                def ready():
                    assert core.poll() is None, f'{label}: supplied Core exited'
                    try:
                        return endpoint.exists() and request(api, 'session.snapshot')
                    except (OSError, AssertionError):
                        return None
                until(ready, f'{label}: supplied Core startup')
                first = request(api, 'workspace.create', {'label': 'Review', 'cwd': str(home), 'focus': True})
                workspace, original = first['workspace']['workspace_id'], first['tab']['tab_id']
                second = request(api, 'tab.create', {'workspace_id': workspace, 'label': 'Target',
                                                   'cwd': str(home), 'focus': False})
                target, pane = second['tab']['tab_id'], second['root_pane']['pane_id']
                context = {'workspace_id': workspace, 'tab_id': target, 'pane_id': pane}
                for plugin in PALETTE_PLUGINS:
                    request(api, 'plugin.link', {'path': str(plugins / plugin), 'enabled': True})
                catalog = request(api, 'ui.command.list')
                commands = [item for item in catalog['commands'] if item['key'] == KEY]
                assert len(commands) == (1 if expected_title else 0), (label, commands)
                command = commands[0] if commands else None
                if command:
                    assert command['title'] == expected_title, command
                    assert command['target'] == 'tab', command
                last_projection = None
                def projected():
                    value = declarations(endpoint)
                    nonlocal last_projection
                    last_projection = value
                    if any(item['plugin_id'] == PALETTE_OWNER and item.get('declaration')
                           for item in value['ui']):
                        return value
                try:
                    projection = until(projected, f'{label}: actual dedicated palette publication')
                except (AssertionError, OSError):
                    (evidence / 'consumer-publication-diagnostics.json').write_text(json.dumps({
                        'logs': request(api, 'plugin.log.list', {'plugin_id': PALETTE_OWNER}),
                        'plugins': request(api, 'plugin.list'), 'projection': last_projection,
                    }, indent=2))
                    raise
                assert projection['boot_id'] == catalog['boot_id']
                exports = [(item['plugin_id'], fragment) for item in projection['ui']
                           if item.get('declaration')
                           for fragment in item['declaration'].get('fragments', [])]
                palettes = [(owner, fragment) for owner, fragment in exports
                            if fragment['name'] == 'command-palette']
                assert len(palettes) == 1 and palettes[0][0] == PALETTE_OWNER, palettes
                palette = palettes[0][1]
                paths = palette_uses(palette['tree'])
                issued = {item['fragment_name']: item for item in catalog['commands']}
                for locator, _, _ in paths:
                    assert locator in issued, (label, 'non-catalog palette Use', locator)
                    matches = [(owner, fragment) for owner, fragment in exports
                               if fragment['name'] == locator]
                    assert len(matches) == 1, (label, 'unresolvable issued Use', locator, matches)
                selected_path = None
                if command:
                    matches = [(owner, fragment) for owner, fragment in exports
                               if fragment['name'] == command['fragment_name']]
                    assert len(matches) == 1 and matches[0][0] == 'trailboss.core', matches
                    candidates = [path for path in paths if path[0] == command['fragment_name']
                                  and path[1].get('workspace_id') == workspace
                                  and path[1].get('tab_id') == target]
                    assert len(candidates) == 1, (label, 'actual consumer lost captured target', paths)
                    selected_path = candidates[0]
                    assert selected_path[2] == (expected_title, 'Review', 'Target'), selected_path
                else:
                    assert not any(route and route[0] in ('Fixture focus', 'Retitled fixture focus')
                                   for _, _, route in paths), (label, paths)
                (evidence / 'projection.json').write_text(json.dumps(projection, indent=2))
                (evidence / 'palette-paths.json').write_text(json.dumps(paths, indent=2))
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
                def controlling_terminal():
                    os.setsid()
                    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
                native = subprocess.Popen([str(client_binary), '--session', session], cwd=home,
                                          env=env, stdin=slave, stdout=slave, stderr=slave,
                                          preexec_fn=controlling_terminal, close_fds=True)
                os.close(slave)
                def pump():
                    while not stop.is_set():
                        try:
                            readable, _, _ = select.select([master], [], [], 0.05)
                            if readable:
                                block = os.read(master, 65536)
                                if not block:
                                    return
                                output.extend(block)
                        except OSError:
                            return
                drain = threading.Thread(target=pump, daemon=True)
                drain.start()
                def native_ready():
                    assert native.poll() is None, (label, 'native client exited', bytes(output))
                    return b'Review' in output
                until(native_ready, f'{label}: current native client rendered session')
                assert request(api, 'session.snapshot')['snapshot']['focused_tab_id'] == original
                palette_offset = len(output)
                os.write(master, b'\x1bOP')
                until(lambda: b'Commands' in output[palette_offset:], f'{label}: F1 opened actual palette')
                if command:
                    # Observe the authoritative title before typing it; query
                    # text itself must not masquerade as a displayed command.
                    until(lambda: expected_title.encode() in output[palette_offset:],
                          f'{label}: actual palette did not display authoritative title')
                    crumbs = []
                    for title in selected_path[2]:
                        offset = len(output)
                        os.write(master, title.encode() + b'\r')
                        crumbs.append(title)
                        header = ('Commands › ' + ' › '.join(crumbs)).encode()
                        until(lambda: header in output[offset:],
                              f'{label}: actual native target navigation did not open {crumbs!r}')
                    os.write(master, b'\r')
                    until(lambda: request(api, 'session.snapshot')['snapshot']['focused_tab_id'] == target,
                          f'{label}: available supplied Core command did not focus captured Target')
                    selection = 'F1; search title/workspace/target and Enter each; Enter owner command'
                else:
                    # Other commands remain available: absence is observed as
                    # an empty native search, not a fabricated empty palette.
                    for query in ('Fixture focus', 'Retitled fixture focus'):
                        offset = len(output)
                        os.write(master, query.encode())
                        until(lambda: b'No matches' in output[offset:],
                              f'{label}: absent command remained selectable for {query!r}')
                        os.write(master, b'\r')
                        assert request(api, 'session.snapshot')['snapshot']['focused_tab_id'] == original
                        os.write(master, b'\x7f' * len(query))
                    os.write(master, b'\x1b')
                    selection = 'F1; both fixture-title searches show No matches; Enter changes no target'
                snapshot = request(api, 'session.snapshot')['snapshot']
                assert native.poll() is None, (label, bytes(output))
                assert digest(binary) == fixture['sha256']
                assert digest(client_binary) == args.client_hash
                assert {item: digest(plugins / item) for item in PALETTE_ARTIFACTS} == args.producer_hashes
                receipt = {
                    'case': label, 'source_revision': fixture['revision'],
                    'server_sha256': fixture['sha256'], 'client_sha256': args.client_hash,
                    'producer_sha256': args.producer_hashes, 'consumer_sha256': args.consumer_hashes,
                    'boot_id': catalog['boot_id'], 'command': command,
                    'captured_context': context, 'initial_tab': original,
                    'focused_tab': snapshot['focused_tab_id'], 'native_selection': selection,
                    'palette_provider': PALETTE_OWNER, 'palette_provider_count': len(palettes),
                    'bun_absent': True,
                }
                (evidence / 'receipt.json').write_text(json.dumps(receipt, indent=2))
                return receipt
            finally:
                if native is not None and native.poll() is None:
                    native.terminate()
                    try:
                        native.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        native.kill()
                        native.wait()
                stop.set()
                if drain:
                    drain.join(timeout=1)
                if master is not None:
                    os.close(master)
                (evidence / 'native.ansi').write_bytes(output)
                if core.poll() is None:
                    try:
                        request(api, 'server.stop')
                        core.wait(timeout=5)
                    except (OSError, AssertionError, subprocess.TimeoutExpired):
                        core.terminate()
                        core.wait(timeout=3)
                server_log = home / f'trailboss/sessions/{session}/trailboss-server.log'
                if server_log.is_file():
                    shutil.copy2(server_log, evidence / 'core.log')


def main():
    parser = argparse.ArgumentParser()
    for name in ('fixture-manifest', 'client', 'producer-root', 'output-dir'):
        parser.add_argument('--' + name, required=True, type=Path)
    args = parser.parse_args()
    for name in ('fixture_manifest', 'client', 'producer_root', 'output_dir'):
        setattr(args, name, getattr(args, name).resolve())
    supplied = json.loads(args.fixture_manifest.read_text())
    args.fixtures = supplied['fixtures']
    assert set(args.fixtures) == {'base', 'added', 'renamed', 'removed'}
    for label, fixture in args.fixtures.items():
        assert digest(Path(fixture['path'])) == fixture['sha256'], (label, 'unverified supplied bytes')
        assert fixture['revision'], (label, 'missing supplied source provenance')
    args.consumer_hashes = {name: digest(Path(__file__).with_name(name)) for name in
                           ('command_definitions.py', 'command_compatibility.py', 'neutral_consumer.py')}
    args.client_hash = digest(args.client)
    args.producer_hashes = {item: digest(args.producer_root / 'plugins' / item)
                            for item in PALETTE_ARTIFACTS}
    receipts = [run_case(args, 'base', None), run_case(args, 'added', 'Fixture focus'),
                run_case(args, 'renamed', 'Retitled fixture focus'), run_case(args, 'removed', None)]
    assert receipts[1]['command']['key'] == receipts[2]['command']['key'] == KEY
    assert receipts[1]['command']['title'] != receipts[2]['command']['title']
    assert {name: digest(Path(__file__).with_name(name)) for name in args.consumer_hashes} == args.consumer_hashes
    assert digest(args.client) == args.client_hash
    assert {item: digest(args.producer_root / 'plugins' / item)
            for item in PALETTE_ARTIFACTS} == args.producer_hashes
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / 'receipts.json').write_text(json.dumps(receipts, indent=2))
    print(json.dumps({'passed_cases': len(receipts), 'receipts': str(args.output_dir / 'receipts.json')}))


if __name__ == '__main__':
    main()
