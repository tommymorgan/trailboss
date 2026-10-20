"""C15 compiled Core add/rename/remove proof using one fixed native consumer.

The parent supplies actual separately compiled Core definition variants. This
runner neither edits authoritative definitions nor registers runtime commands.
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
import sys
import tempfile
import termios
import threading
import time

from command_compatibility import declarations, digest, request


KEY = {'kind': 'core', 'action_id': 'host:focus'}
PRODUCERS = ('agent-tree/agent-tree', 'agent-tree/stock-ui', 'sidebar-menus/sidebar-menus')


def until(predicate, message, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        value = predicate()
        if value:
            return value
        assert time.monotonic() < deadline, message
        time.sleep(0.01)


def run_case(args, label, expected_title, stale_fragment=None):
    binary = getattr(args, label)
    evidence = args.output_dir / label
    evidence.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='command-definitions-') as temporary:
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
        assert {item: digest(plugins / item) for item in PRODUCERS} == args.producer_hashes
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
                    assert core.poll() is None, f'{label}: actual Core exited'
                    try:
                        return endpoint.exists() and request(api, 'session.snapshot')
                    except (OSError, AssertionError):
                        return None
                until(ready, f'{label}: actual Core startup')
                first = request(api, 'workspace.create', {'label': 'Review', 'cwd': str(home), 'focus': True})
                workspace, original = first['workspace']['workspace_id'], first['tab']['tab_id']
                second = request(api, 'tab.create', {'workspace_id': workspace, 'label': 'Target',
                                                   'cwd': str(home), 'focus': False})
                target, pane = second['tab']['tab_id'], second['root_pane']['pane_id']
                context = {'workspace_id': workspace, 'tab_id': target, 'pane_id': pane}
                for plugin in ('agent-tree', 'stock-ui', 'sidebar-menus'):
                    request(api, 'plugin.link', {'path': str(plugins / plugin), 'enabled': True})
                consumer = home / 'consumer'
                consumer.mkdir()
                config = consumer / 'config.json'
                config.write_text(json.dumps({'api': str(api), 'context': context,
                                               'stale_fragment': stale_fragment}))
                (consumer / 'trailboss-plugin.toml').write_text(
                    'id = "fixture.definition-consumer"\nname = "Definition consumer"\nversion = "0.1.0"\n'
                    'min_herdr_version = "0.9.0"\n[[ui]]\nid = "consumer"\nsurface = "sidebar"\n'
                    f'command = [{json.dumps(sys.executable)}, {json.dumps(str(args.consumer))}, '
                    f'{json.dumps(str(config))}]\nstream = true\n')
                request(api, 'plugin.link', {'path': str(consumer), 'enabled': True})
                catalog = request(api, 'ui.command.list')
                commands = [item for item in catalog['commands'] if item['key'] == KEY]
                assert len(commands) == (1 if expected_title else 0), (label, commands)
                command = commands[0] if commands else None
                if command:
                    assert command['title'] == expected_title, command
                last_projection = None
                def projected():
                    value = declarations(endpoint)
                    nonlocal last_projection
                    last_projection = value
                    if any(item['plugin_id'] == 'fixture.definition-consumer' and item.get('declaration')
                           for item in value['ui']):
                        return value
                try:
                    projection = until(projected, f'{label}: actual fixed consumer publication')
                except (AssertionError, OSError):
                    diagnostics = {
                        'plugin_ui': request(api, 'plugin.ui.list'),
                        'plugins': request(api, 'plugin.list'),
                        'endpoint_projection': last_projection,
                    }
                    (evidence / 'consumer-publication-diagnostics.json').write_text(
                        json.dumps(diagnostics, indent=2))
                    raise
                assert projection['boot_id'] == catalog['boot_id']
                exports = [(item['plugin_id'], fragment) for item in projection['ui']
                           if item.get('declaration')
                           for fragment in item['declaration'].get('fragments', [])]
                if command:
                    matches = [(owner, fragment) for owner, fragment in exports
                               if fragment['name'] == command['fragment_name']]
                    assert len(matches) == 1 and matches[0][0] == 'trailboss.core', matches
                if stale_fragment:
                    assert not any(fragment['name'] == stale_fragment for _, fragment in exports)
                palette = next(fragment for owner, fragment in exports
                               if owner == 'fixture.definition-consumer' and fragment['name'] == 'command-palette')
                wanted = command['fragment_name'] if command else stale_fragment
                children = palette['tree']['children']
                assert len(children) == (1 if wanted else 0), palette
                if wanted:
                    row = children[0]
                    assert row['type'] == 'row'
                    assert all(row['context'][key] == value for key, value in context.items())
                    assert len(row['children']) == 1
                    assert row['children'][0]['type'] == 'label'
                    assert row['children'][0]['text'] == 'Captured Target'
                    assert row['menu']['type'] == 'column'
                    assert len(row['menu']['children']) == 1
                    assert row['menu']['children'][0]['type'] == 'use'
                    assert row['menu']['children'][0]['name'] == wanted
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
                until(native_ready, f'{label}: native client rendered actual session')
                assert request(api, 'session.snapshot')['snapshot']['focused_tab_id'] == original
                os.write(master, b'\x1bOP')
                if wanted:
                    until(lambda: b'Captured Target' in output,
                          f'{label}: native palette must render captured submenu')
                    os.write(master, b'\r')
                if expected_title:
                    until(lambda: expected_title.encode() in output,
                          f'{label}: native palette must render the generated title')
                else:
                    # No selectable command exists, including when the producer
                    # deliberately retains an old boot's unresolved Use.
                    time.sleep(0.2)
                os.write(master, b'\r')
                if command:
                    until(lambda: request(api, 'session.snapshot')['snapshot']['focused_tab_id'] == target,
                          f'{label}: selected compiled Core command must actually focus captured Target')
                else:
                    time.sleep(0.3)
                    assert request(api, 'session.snapshot')['snapshot']['focused_tab_id'] == original
                snapshot = request(api, 'session.snapshot')['snapshot']
                assert native.poll() is None, (label, bytes(output))
                receipt = {'case': label, 'source_revision': getattr(args, label + '_revision'),
                           'server_sha256': digest(binary), 'client_sha256': args.client_hash,
                           'producer_sha256': args.producer_hashes, 'consumer_sha256': args.consumer_hashes,
                           'boot_id': catalog['boot_id'], 'command': command,
                           'captured_context': context, 'initial_tab': original,
                           'focused_tab': snapshot['focused_tab_id'], 'stale_fragment': stale_fragment,
                           'native_selection': 'F1, Enter submenu, Enter command' if wanted else 'F1, Enter empty',
                           'bun_absent': True}
                (evidence / 'projection.json').write_text(json.dumps(projection, indent=2))
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


def main():
    parser = argparse.ArgumentParser()
    for name in ('base', 'added', 'renamed', 'removed', 'client', 'producer-root', 'output-dir'):
        parser.add_argument('--' + name, required=True, type=Path)
    for name in ('base', 'added', 'renamed', 'removed'):
        parser.add_argument('--' + name + '-revision', required=True)
    args = parser.parse_args()
    for name in ('base', 'added', 'renamed', 'removed', 'client', 'producer_root', 'output_dir'):
        setattr(args, name, getattr(args, name).resolve())
    args.consumer = Path(__file__).with_name('command_definition_consumer.py').resolve()
    args.consumer_hashes = {name: digest(args.consumer.with_name(name)) for name in
                           ('command_definition_consumer.py', 'command_compatibility.py', 'neutral_consumer.py')}
    args.client_hash = digest(args.client)
    args.producer_hashes = {item: digest(args.producer_root / 'plugins' / item) for item in PRODUCERS}
    receipts = [run_case(args, 'base', None), run_case(args, 'added', 'Fixture focus'),
                run_case(args, 'renamed', 'Retitled fixture focus')]
    assert receipts[1]['command']['key'] == receipts[2]['command']['key'] == KEY
    assert receipts[1]['command']['title'] != receipts[2]['command']['title']
    receipts.append(run_case(args, 'removed', None, receipts[1]['command']['fragment_name']))
    assert {name: digest(args.consumer.with_name(name)) for name in args.consumer_hashes} == args.consumer_hashes
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / 'receipts.json').write_text(json.dumps(receipts, indent=2))
    print(json.dumps({'passed_cases': len(receipts), 'receipts': str(args.output_dir / 'receipts.json')}))


if __name__ == '__main__':
    main()
