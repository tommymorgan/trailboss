"""C14 actual old-generation/package boundary. Parent supplies built artifacts.

No application imports, source inspections, producer substitutes, or Bun fallback.
The unchanged pre-discovery neutral_consumer.py decodes actual generation-1 frames.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time

from neutral_consumer import Reader, read_exact, send


def request(path, method, params=None):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(5)
        connection.connect(str(path))
        connection.sendall((json.dumps({'id': 'compatibility-probe', 'method': method,
                                       'params': params or {}}) + '\n').encode())
        data = bytearray()
        while b'\n' not in data:
            part = connection.recv(65536)
            assert part, 'Core closed the actual API without a response'
            data.extend(part)
        reply = json.loads(data.split(b'\n')[0])
        assert 'result' in reply, (method, reply)
        return reply['result']


def declarations(path):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(8)
        connection.connect(str(path))
        send(connection, 20, 'endpoint.hello.v1', json.dumps({
            'generation': 1, 'cell_width_px': 0, 'cell_height_px': 0,
            'surface_size': {'cols': 80, 'rows': 24}, 'pixel_mouse': False,
            'direct_graphics': False, 'endpoint_keybindings': False, 'mouse_capture': False,
            'surface_active': False, 'surface_reuse': False, 'surface_delta': False,
            'surface_scroll': False, 'snapshot_codecs': ['shell.snapshot.v1'],
            'surface_codecs': ['shell.surface.v1'], 'input_codecs': ['shell.input.semantic.v1'],
            'blob_codecs': ['shell.blob.v1']}))
        while True:
            size = struct.unpack('<I', read_exact(connection, 4))[0]
            assert size <= 2 * 1024 * 1024
            reader = Reader(read_exact(connection, size))
            if reader.integer() != 20:
                continue
            key, payload = reader.text(), reader.text()
            reader.finish()
            if key == 'endpoint.plugin-ui.v1':
                value = json.loads(payload)
                for item in value['ui']:
                    if item['plugin_id'] == 'tommymorgan.sidebar-menus' and item.get('declaration'):
                        if any(fragment['name'] == 'commands.confirm-space'
                               for fragment in item['declaration'].get('fragments', [])):
                            return value


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_case(args, binary, annotation, current):
    label = ('candidate' if current else 'old-core') + ('-annotated' if annotation else '-original')
    evidence = args.output_dir / label
    evidence.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='command-compatibility-') as temporary:
        home = Path(temporary)
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(('HERDR_', 'TRAILBOSS_'))}
        for key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_DATA_HOME',
                    'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
            env[key] = str(home)
        env.update(PATH=args.no_bun_path, SHELL='/bin/sh',
                   TRAILBOSS_CONFIG_PATH=str(home / 'config.toml'),
                   TRAILBOSS_STARTUP_CWD=str(home))
        assert shutil.which('bun', path=env['PATH']) is None, 'Bun must genuinely be absent from this Core/producer PATH'
        plugins = home / 'plugins'
        shutil.copytree(args.producer_root / 'plugins', plugins)
        menus_manifest = plugins / 'sidebar-menus/trailboss-plugin.toml'
        manifest = menus_manifest.read_text()
        annotation_line = 'confirmation_fragment = "commands.confirm-space"'
        manifest = '\n'.join(line for line in manifest.splitlines()
                             if not line.strip().startswith('confirmation_fragment')) + '\n'
        if annotation:
            assert 'id = "close-space"' in manifest
            manifest = manifest.replace('id = "close-space"', 'id = "close-space"\n' + annotation_line, 1)
        menus_manifest.write_text(manifest)
        name = 'compatibility'
        api = home / f'trailboss/sessions/{name}/trailboss.sock'
        endpoint = home / f'trailboss/sessions/{name}/trailboss-client.sock'
        with (evidence / 'core.stderr').open('wb') as stderr:
            process = subprocess.Popen([str(binary), 'server', '--session', name], cwd=home,
                                       env=env, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.DEVNULL, stderr=stderr)
            try:
                deadline = time.monotonic() + 10
                while True:
                    assert process.poll() is None, f'{label}: actual Core exited during startup'
                    try:
                        if endpoint.exists():
                            request(api, 'session.snapshot')
                            break
                    except (OSError, AssertionError):
                        pass
                    assert time.monotonic() < deadline, f'{label}: Core startup deadline'
                    time.sleep(0.01)
                first = request(api, 'workspace.create', {'label': 'Review', 'cwd': str(home), 'focus': True})
                workspace = first['workspace']['workspace_id']
                original = first['tab']['tab_id']
                second = request(api, 'tab.create', {'workspace_id': workspace, 'label': 'Target',
                                                     'cwd': str(home), 'focus': False})
                target, pane = second['tab']['tab_id'], second['root_pane']['pane_id']
                for plugin in ('agent-tree', 'stock-ui', 'sidebar-menus'):
                    request(api, 'plugin.link', {'path': str(plugins / plugin), 'enabled': True})
                tone = home / 'semantic-tone'
                tone.mkdir()
                (tone / 'trailboss-plugin.toml').write_text(
                    'id = "fixture.semantic-tone"\nname = "Semantic tone"\nversion = "0.1.0"\n'
                    'min_herdr_version = "0.9.0"\n[[ui]]\nid = "tone"\nsurface = "sidebar"\n'
                    f'command = [{json.dumps(sys.executable)}, "producer.py"]\nstream = true\n')
                declaration = {'herdr': 'widget_tree', 'version': 1, 'zones': [
                    {'zone': 'footer', 'tree': {'type': 'label', 'text': 'Neutral tone',
                                              'role': 'muted', 'tone': 'fg-cyan'}}]}
                (tone / 'producer.py').write_text(
                    f'import sys\nprint({json.dumps(json.dumps(declaration))}, flush=True)\n'
                    'for line in sys.stdin:\n pass\n')
                request(api, 'plugin.link', {'path': str(tone), 'enabled': True})
                request(api, 'pane.send_text', {'pane_id': pane,
                    'text': "printf '\\033[2J\\033[H\\033[1;38;2;17;34;51mneutral-%s\\033[0m\\033[4;7H\\033[6 q' live; read compatibility_hold\n"})
                # This file predates discovery and remains unchanged. Its decoder
                # rejects trailing binary fields and sends the original owner request.
                decoded_path = evidence / 'old-consumer.json'
                consumer = subprocess.run([sys.executable, str(Path(__file__).with_name('neutral_consumer.py')),
                                           str(endpoint), target, str(decoded_path)], env=env,
                                          capture_output=True, timeout=15)
                assert consumer.returncode == 0, (label, consumer.stderr.decode())
                decoded = json.loads(decoded_path.read_text())
                assert decoded['snapshots'][0]['focused_tab_id'] == original
                assert decoded['snapshots'][-1]['focused_tab_id'] == target
                assert decoded['action']['plugin_id'] == 'tommymorgan.agent-tree'
                assert decoded['action']['action_id'] == 'focus-tab'
                assert decoded['action']['context']['tab_id'] == target
                assert any('neutral-live' in ''.join(cell['symbol'] for cell in frame['cells'])
                           for frame in decoded['surfaces'])
                projection = declarations(endpoint)
                all_fragments = [fragment for item in projection['ui'] if item.get('declaration')
                                 for fragment in item['declaration'].get('fragments', [])]
                issued = [fragment['name'] for fragment in all_fragments
                          if fragment['name'].startswith('trailboss.command.v1:')]
                if current:
                    discovered = request(api, 'ui.command.list')
                    assert discovered['boot_id'] == projection['boot_id']
                    assert issued, 'candidate must publish generated fragments alongside unchanged producers'
                else:
                    assert not issued, 'old Core artifact must genuinely predate discovery'
                owner_export = next(fragment for item in projection['ui']
                    if item['plugin_id'] == 'tommymorgan.sidebar-menus' and item.get('declaration')
                    for fragment in item['declaration'].get('fragments', [])
                    if fragment['name'] == 'commands.confirm-space')
                affirmative = [node for node in owner_export['tree']['children']
                               if node.get('action') == 'close-space']
                assert len(affirmative) == 1
                other = request(api, 'workspace.create', {'label': 'Other', 'cwd': str(home), 'focus': True})
                action = request(api, 'plugin.action.invoke', {
                    'plugin_id': 'tommymorgan.sidebar-menus', 'action_id': affirmative[0]['action'],
                    'context': {'workspace_id': workspace, 'tab_id': target, 'pane_id': pane,
                                'invocation_source': 'plugin_ui'}})
                deadline = time.monotonic() + 5
                while True:
                    snapshot = request(api, 'session.snapshot')['snapshot']
                    logs = request(api, 'plugin.log.list', {'plugin_id': 'tommymorgan.sidebar-menus'})['logs']
                    log = next(log for log in logs if log['log_id'] == action['log']['log_id'])
                    if log['status'] != 'running':
                        assert log['status'] == 'succeeded', log
                        assert not any(item['workspace_id'] == workspace for item in snapshot['workspaces'])
                        assert any(item['workspace_id'] == other['workspace']['workspace_id']
                                   for item in snapshot['workspaces'])
                        break
                    assert time.monotonic() < deadline, f'{label}: actual original close-space owner did not finish'
                    time.sleep(0.01)
                assert process.poll() is None, 'client completion must leave owned Core alive'
                receipt = {'case': label, 'revision': args.revision if current else args.old_revision,
                           'binary_sha256': digest(binary), 'boot_id': projection['boot_id'],
                           'producer_sha256': args.producer_hashes, 'bun_absent': True,
                           'generation': 1, 'focus_owner': decoded['action'],
                           'closed_workspace': workspace, 'preserved_workspace': other['workspace']['workspace_id'],
                           'close_owner': 'tommymorgan.sidebar-menus', 'close_log': log,
                           'generated_fragment_count': len(issued)}
                (evidence / 'receipt.json').write_text(json.dumps(receipt, indent=2))
                return receipt
            finally:
                if process.poll() is None:
                    try:
                        request(api, 'server.stop')
                        process.wait(timeout=5)
                    except (OSError, AssertionError, subprocess.TimeoutExpired):
                        process.terminate()
                        try:
                            process.wait(timeout=3)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()


def main():
    parser = argparse.ArgumentParser()
    for name in ('candidate', 'old-core', 'producer-root', 'output-dir'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--no-bun-path', required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--old-revision', required=True)
    args = parser.parse_args()
    args.candidate, args.old_core, args.producer_root = [path.resolve() for path in
                                                       (args.candidate, args.old_core, args.producer_root)]
    artifacts = ['agent-tree/agent-tree', 'agent-tree/stock-ui', 'sidebar-menus/sidebar-menus']
    args.producer_hashes = {}
    for artifact in artifacts:
        path = args.producer_root / 'plugins' / artifact
        assert path.is_file() and os.access(path, os.X_OK), f'missing real compiled producer: {path}'
        args.producer_hashes[artifact] = digest(path)
    receipts = [run_case(args, binary, annotation, current)
                for binary, current in ((args.candidate, True), (args.old_core, False))
                for annotation in (False, True)]
    (args.output_dir / 'receipts.json').write_text(json.dumps(receipts, indent=2))
    print(json.dumps({'passed_cases': len(receipts), 'receipts': str(args.output_dir / 'receipts.json')}))


if __name__ == '__main__':
    main()
