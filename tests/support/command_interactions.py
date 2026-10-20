"""C16: 28 fresh, owned actual-Core/native-PTY interaction examples.

Parent builds and runs this driver. No builds, installs, source rewriting, runtime
command registration, direct API substitutes for menu actions, or user sessions.
API calls arrange targets/change focus and observe effects; all commands under
proof are selected with F1, a typed query and Enter in the actual native client.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import traceback
import unicodedata

from command_compatibility import digest, request
from command_definitions import PRODUCERS, until
from neutral_consumer import Reader, read_exact, send


OWNER = 'tommymorgan.sidebar-menus'
CONSUMER = 'fixture.interaction-consumer'
EXAMPLES = (
    ('new-space-valid', 'palette new-space with a valid name and submit'),
    ('new-space-blank', 'palette new-space with blank input'),
    ('new-space-cancel', 'palette new-space cancellation'),
    ('new-tab-valid', 'palette new-tab with a valid name and submit'),
    ('new-tab-blank', 'palette new-tab with blank input'),
    ('new-tab-cancel', 'palette new-tab cancellation'),
    ('rename-space-valid', 'rename-space with valid input after focus changes'),
    ('rename-space-blank', 'rename-space with blank input'),
    ('rename-space-cancel', 'rename-space cancellation'),
    ('rename-tab-valid', 'rename-tab with valid input after focus changes'),
    ('rename-tab-blank', 'rename-tab with blank input'),
    ('rename-tab-cancel', 'rename-tab cancellation'),
    ('close-tab-cancel', 'close-tab confirmation cancelled'),
    ('close-tab-accept', 'close-tab confirmation accepted'),
    ('close-space-cancel', 'plugin close-space confirmation cancelled'),
    ('close-space-accept', 'plugin close-space confirmation accepted'),
    ('previous-empty', 'Previous tab without a prior live visit'),
    ('previous-global', 'A active, C previously visited, menu opened for B, then Previous tab from the global surface'),
    ('rename-branch', 'Rename from the resource branch captured for B after navigating to C'),
    ('new-native-scope', 'host:new while a resource menu for another workspace is open'),
    ('confirmation-pending', 'close-space confirmation export not published yet'),
    ('confirmation-missing-or-wrong', 'close-space confirmation export missing or containing another affirmative action'),
    ('confirmation-context', 'close-space export embeds a nondefault target context'),
    ('confirmation-ambiguous', 'two same-owner declarations export the configured confirmation name'),
    ('confirmation-recovery', 'close-space owner publishes its valid confirmation export after startup'),
    ('detach', 'Detach'),
    ('targeted-focus', 'targeted focus'),
    ('restart-idle-agents', 'restart-idle-agents'),
)


def core_key(action, mode=None):
    value = {'kind': 'core', 'action_id': 'host:' + action}
    if mode:
        value['mode'] = mode
    return value


CLOSE_SPACE = {'kind': 'plugin', 'plugin_id': OWNER, 'action_id': 'close-space'}


def projection(endpoint):
    """Read the actual unmodified generation-1 endpoint frame, without requiring
    a valid sidebar confirmation export (negative cases intentionally lack it).
    """
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(8)
        connection.connect(str(endpoint))
        send(connection, 20, 'endpoint.hello.v1', json.dumps({
            'generation': 1, 'cell_width_px': 0, 'cell_height_px': 0,
            'surface_size': {'cols': 100, 'rows': 30}, 'pixel_mouse': False,
            'direct_graphics': False, 'endpoint_keybindings': False, 'mouse_capture': False,
            'surface_active': False, 'surface_reuse': False, 'surface_delta': False,
            'surface_scroll': False, 'snapshot_codecs': ['shell.snapshot.v1'],
            'surface_codecs': ['shell.surface.v1'], 'input_codecs': ['shell.input.semantic.v1'],
            'blob_codecs': ['shell.blob.v1']}))
        while True:
            size = struct.unpack('<I', read_exact(connection, 4))[0]
            assert size <= 2 * 1024 * 1024, 'oversized generation-1 frame'
            reader = Reader(read_exact(connection, size))
            if reader.integer() != 20:
                continue
            key, payload = reader.text(), reader.text()
            reader.finish()
            if key == 'endpoint.plugin-ui.v1':
                return json.loads(payload)


def exports(value):
    return [(item['plugin_id'], fragment) for item in value['ui']
            if item.get('declaration') for fragment in item['declaration'].get('fragments', [])]


def native_screen(data, rows=30, cols=110):
    """Reconstruct the current native text screen, including differential writes.

    F1 can reopen an identical root without emitting any new terminal cells.
    This observes current display state rather than requiring redundant output.
    Styling, OSC metadata and terminal-mode switches do not occupy cells.
    """
    cells = [[' '] * cols for _ in range(rows)]
    x = y = 0
    token = re.compile(
        r'\x1b\][^\x1b\x07]*(?:\x07|\x1b\\)|'
        r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b[()][^\x1b]|\x1b[=>]|.',
        re.DOTALL)
    for match in token.finditer(data.decode('utf8', errors='replace')):
        value = match.group()
        if value.startswith('\x1b['):
            body, command = value[2:-1], value[-1]
            if body.startswith(('?', '>', '<', '=')):
                continue
            parameters = [int(item) if item else 0 for item in body.split(';')]
            amount = parameters[0] or 1
            if command in ('H', 'f'):
                y = min(rows - 1, max(0, amount - 1))
                x = min(cols - 1, max(0, (parameters[1] if len(parameters) > 1 else 1) - 1))
            elif command == 'A':
                y = max(0, y - amount)
            elif command == 'B':
                y = min(rows - 1, y + amount)
            elif command == 'C':
                x = min(cols - 1, x + amount)
            elif command == 'D':
                x = max(0, x - amount)
            elif command == 'G':
                x = min(cols - 1, max(0, amount - 1))
            elif command == 'd':
                y = min(rows - 1, max(0, amount - 1))
            elif command == 'J' and parameters[0] in (2, 3):
                cells = [[' '] * cols for _ in range(rows)]
            elif command == 'K':
                start = 0 if parameters[0] in (1, 2) else x
                end = cols if parameters[0] in (0, 2) else x + 1
                cells[y][start:end] = [' '] * (end - start)
            continue
        if value.startswith('\x1b'):
            continue
        if value == '\r':
            x = 0
        elif value == '\n':
            y = min(rows - 1, y + 1)
        elif value == '\b':
            x = max(0, x - 1)
        elif value == '\t':
            x = min(cols - 1, ((x // 8) + 1) * 8)
        elif not unicodedata.category(value).startswith('C') and not unicodedata.combining(value):
            width = 2 if unicodedata.east_asian_width(value) in ('W', 'F') else 1
            if x < cols:
                cells[y][x] = value
                if width == 2 and x + 1 < cols:
                    cells[y][x + 1] = ''
            x = min(cols, x + width)
    return [''.join(row).rstrip() for row in cells]


class OwnedEndpointTap:
    """Transparent byte relay for this fixture's owned native endpoint only.

    It never creates an operation, changes a payload or replays a request.
    Native currently discards endpoint response chunks instead of rendering
    restart summaries, so the real correlated reply is the observation seam.
    """
    def __init__(self, endpoint, evidence):
        self.upstream = endpoint.with_name('owned-upstream.sock')
        endpoint.rename(self.upstream)
        self.evidence = evidence / 'native-endpoint'
        self.evidence.mkdir()
        self.events, self.errors, self.connections, self.workers = [], [], [], []
        self.stopped = threading.Event()
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(str(endpoint))
        self.listener.listen()
        self.listener.settimeout(0.1)
        self.acceptor = threading.Thread(target=self.accept, daemon=True)
        self.acceptor.start()

    def accept(self):
        while not self.stopped.is_set():
            try:
                client, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            core = socket.socket(socket.AF_UNIX)
            try:
                core.connect(str(self.upstream))
            except OSError as error:
                self.errors.append(str(error))
                client.close()
                core.close()
                continue
            index = len(self.workers)
            self.connections.extend((client, core))
            worker = threading.Thread(target=self.relay, args=(index, client, core), daemon=True)
            self.workers.append(worker)
            worker.start()

    def record(self, index, direction, buffer):
        while len(buffer) >= 4:
            size = struct.unpack('<I', buffer[:4])[0]
            assert size <= 2 * 1024 * 1024, 'oversized actual native endpoint frame'
            if len(buffer) < size + 4:
                return
            frame = bytes(buffer[4:size + 4])
            del buffer[:size + 4]
            reader = Reader(frame)
            tag = reader.integer()
            event = None
            if direction == 'client' and tag == 15:
                boot, request_text = reader.text(), reader.text()
                reader.finish()
                event = {'connection': index, 'direction': direction, 'boot_id': boot,
                         'request': json.loads(request_text)}
            elif direction == 'core' and tag == 18:
                boot, request_id, final = reader.text(), reader.text(), reader.byte()
                assert final in (0, 1), 'invalid actual response final_chunk'
                data = bytes(reader.take(reader.integer()))
                reader.finish()
                event = {'connection': index, 'direction': direction, 'boot_id': boot,
                         'request_id': request_id, 'final_chunk': bool(final), 'data_hex': data.hex()}
            if event is not None:
                self.events.append(event)
                with (self.evidence / 'operations.jsonl').open('a') as output:
                    output.write(json.dumps(event) + '\n')

    def relay(self, index, client, core):
        buffers = {'client': bytearray(), 'core': bytearray()}
        try:
            with (self.evidence / f'{index}-client.bin').open('wb') as client_bytes, (
                    self.evidence / f'{index}-core.bin').open('wb') as core_bytes:
                routes = {client: (core, 'client', client_bytes), core: (client, 'core', core_bytes)}
                while not self.stopped.is_set():
                    ready, _, _ = select.select([client, core], [], [], 0.1)
                    for source in ready:
                        block = source.recv(65536)
                        if not block:
                            return
                        destination, direction, output = routes[source]
                        output.write(block)
                        output.flush()
                        # Forward the exact read bytes, without reconstructing
                        # messages from the independently decoded observation.
                        destination.sendall(block)
                        buffers[direction].extend(block)
                        self.record(index, direction, buffers[direction])
        except (BrokenPipeError, ConnectionResetError):
            # The independent projection reader intentionally closes after its
            # endpoint UI frame. Native EOF is likewise not a forwarding retry.
            pass
        except (OSError, ValueError, AssertionError) as error:
            if not self.stopped.is_set():
                self.errors.append(str(error))
        finally:
            client.close()
            core.close()

    def restart_result(self):
        assert not self.errors, self.errors
        events = list(self.events)
        requests = [event for event in events if event['direction'] == 'client'
                    and event['request']['method'] == 'agent.restart_idle']
        assert len(requests) <= 1, ('native issued duplicate restart requests', requests)
        if not requests:
            return None
        issued = requests[0]
        chunks = [event for event in events if event['direction'] == 'core'
                  and event['connection'] == issued['connection']
                  and event['boot_id'] == issued['boot_id']
                  and event['request_id'] == issued['request']['id']]
        if not chunks or not chunks[-1]['final_chunk']:
            return None
        assert sum(chunk['final_chunk'] for chunk in chunks) == 1, chunks
        response = json.loads(b''.join(bytes.fromhex(chunk['data_hex']) for chunk in chunks))
        assert response['id'] == issued['request']['id'], response
        assert 'error' not in response, response
        assert response['result']['type'] == 'agent_restart_idle', response
        assert response['result']['results'] == [], response
        return {'native_request': issued, 'correlated_core_response': response,
                'response_chunks': chunks, 'request_count': len(requests)}

    def close(self):
        self.stopped.set()
        self.listener.close()
        self.acceptor.join(timeout=2)
        for connection in self.connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            connection.close()
        for worker in self.workers:
            worker.join(timeout=2)
        (self.evidence / 'observer-errors.json').write_text(json.dumps(self.errors, indent=2))


class OwnedSession:
    def __init__(self, args, index, name):
        self.args, self.name = args, name
        self.evidence = args.output_dir / f'{index:02d}-{name}'
        self.evidence.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix='c16-native-')
        self.home = Path(self.temporary.name)
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(('HERDR_', 'TRAILBOSS_'))}
        for key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_DATA_HOME',
                    'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
            self.env[key] = str(self.home)
        self.env.update(PATH='', SHELL='/bin/sh', TERM='xterm-256color',
                        TRAILBOSS_CONFIG_PATH=str(self.home / 'config.toml'),
                        TRAILBOSS_STARTUP_CWD=str(self.home))
        self.bundle = self.home / 'bundle'
        self.bundle.mkdir()
        self.binary = self.bundle / 'trailboss'
        shutil.copy2(args.candidate, self.binary)
        self.plugins = self.bundle / 'plugins'
        source = args.producer_root
        if (source / 'plugins').is_dir():
            source = source / 'plugins'
        shutil.copytree(source, self.plugins)
        assert digest(self.binary) == args.candidate_hash
        self.producer_hashes = {item: digest(self.plugins / item) for item in PRODUCERS}
        assert self.producer_hashes == args.producer_hashes
        manifest = self.plugins / 'sidebar-menus/trailboss-plugin.toml'
        original = manifest.read_text()
        self.save('sidebar-original.toml', original)
        if 'confirmation_fragment' not in original:
            original = original.replace('id = "close-space"',
                                        'id = "close-space"\nconfirmation_fragment = "commands.confirm-space"', 1)
        manifest.write_text(original)
        self.owner_manifest = manifest
        self.owner_original = original
        self.save('sidebar-staged.toml', original)
        self.api = self.home / 'trailboss/sessions/interactions/trailboss.sock'
        self.endpoint = self.home / 'trailboss/sessions/interactions/trailboss-client.sock'
        self.output = bytearray()
        self.stop = threading.Event()
        self.native = self.master = self.drain = self.core = None
        self.tap = None
        self.sinks = {}
        self.receipt = {'example': index, 'case': name, 'revision': args.revision,
                        'candidate_sha256': args.candidate_hash,
                        'producer_sha256': self.producer_hashes,
                        'consumer_sha256': args.consumer_hashes,
                        'bun_absent': True, 'events': []}
        self.stderr = (self.evidence / 'core.stderr').open('wb')

    def save(self, name, value):
        path = self.evidence / name
        path.write_text(value if isinstance(value, str) else json.dumps(value, indent=2))

    def api_call(self, method, params=None):
        try:
            result = request(self.api, method, params)
        except (OSError, AssertionError) as error:
            with (self.evidence / 'api.jsonl').open('a') as output:
                output.write(json.dumps({'method': method, 'params': params or {},
                                         'error': str(error)}) + '\n')
            raise
        with (self.evidence / 'api.jsonl').open('a') as output:
            output.write(json.dumps({'method': method, 'params': params or {}, 'result': result}) + '\n')
        return result

    def start(self):
        self.core = subprocess.Popen([str(self.binary), 'server', '--session', 'interactions'],
                                     cwd=self.home, env=self.env, stdin=subprocess.DEVNULL,
                                     stdout=subprocess.DEVNULL, stderr=self.stderr)
        def ready():
            assert self.core.poll() is None, 'owned actual Core exited at startup'
            try:
                return self.endpoint.exists() and self.api_call('session.snapshot')
            except (OSError, AssertionError):
                return None
        until(ready, 'owned Core startup')
        a = self.api_call('workspace.create', {'label': 'Review', 'cwd': str(self.home), 'focus': True})
        self.workspace = a['workspace']['workspace_id']
        self.a = a['tab']['tab_id']
        b = self.api_call('tab.create', {'workspace_id': self.workspace, 'label': 'Target',
                                       'cwd': str(self.home), 'focus': False})
        self.b = b['tab']['tab_id']
        c = self.api_call('workspace.create', {'label': 'Other', 'cwd': str(self.home), 'focus': False})
        self.other = c['workspace']['workspace_id']
        self.c = c['tab']['tab_id']
        self.context = {'workspace_id': self.workspace, 'tab_id': self.b,
                        'pane_id': b['root_pane']['pane_id']}
        self.other_context = {'workspace_id': self.other, 'tab_id': self.c,
                              'pane_id': c['root_pane']['pane_id']}
        self.receipt.update(captured_context=self.context, other_context=self.other_context,
                            initial_tab=self.a)
        # Raw byte witnesses detect query/name/control leakage, not just lines
        # subsequently echoed by a shell. They never receive synthetic success.
        for created in (a, b, c):
            pane = created['root_pane']['pane_id']
            sink = self.home / ('input-' + pane)
            code = ('import os,tty; tty.setraw(0); print("C16_WITNESS_READY",flush=True); '
                    f'f=open({str(sink)!r},"ab",buffering=0); '
                    '\nwhile True:\n b=os.read(0,4096)\n if not b: break\n f.write(b)')
            command = shlex.quote(sys.executable) + ' -u -c ' + shlex.quote(code) + '\n'
            self.api_call('pane.send_text', {'pane_id': pane, 'text': command})
            until(lambda: 'C16_WITNESS_READY' in self.api_call(
                'pane.read', {'pane_id': pane, 'source': 'visible'})['read']['text'] and sink.exists(),
                'actual owned pane byte witness starts')
            self.sinks[pane] = sink
        self.api_call('tab.focus', {'tab_id': self.a})

    def install(self, keys, owner_mode=None):
        self.config = self.home / 'interaction-config.json'
        self.configuration = {'api': str(self.api), 'context': self.context, 'keys': keys,
                              'catalog_file': str(self.evidence / 'consumer-catalog.json'),
                              'unexpected_input': str(self.evidence / 'wrong-owner-input.jsonl'),
                              'owner_mode': owner_mode or 'valid',
                              'replacement_context': self.other_context}
        self.config.write_text(json.dumps(self.configuration))
        if owner_mode:
            # Preserve the entire original manifest/action; replace only this
            # owned copy's ordinary UI producer. No test-only runtime seam.
            base = self.owner_original.split('[[ui]]', 1)[0]
            count = 2 if owner_mode == 'ambiguous' else 1
            for number in range(count):
                owner_config = self.config
                if number:
                    owner_config = self.home / f'duplicate-owner-{number}.json'
                    owner_config.write_text(json.dumps({**self.configuration, 'priority': 1000000}))
                base += ('\n[[ui]]\nid = "menus' + str(number) + '"\nsurface = "sidebar"\ncommand = ' +
                         json.dumps([sys.executable, str(self.args.consumer), 'owner', str(owner_config)]) +
                         '\nstream = true\n')
            self.owner_manifest.write_text(base)
            self.save('sidebar-staged.toml', base)
        for plugin in ('agent-tree', 'stock-ui', 'sidebar-menus'):
            self.api_call('plugin.link', {'path': str(self.plugins / plugin), 'enabled': True})
        consumer = self.home / 'consumer'
        consumer.mkdir()
        manifest = ('id = "' + CONSUMER + '"\nname = "Interaction consumer"\nversion = "0.1.0"\n'
                    'min_herdr_version = "0.9.0"\n[[ui]]\nid = "consumer"\nsurface = "sidebar"\ncommand = ' +
                    json.dumps([sys.executable, str(self.args.consumer), 'consumer', str(self.config)]) +
                    '\nstream = true\n')
        (consumer / 'trailboss-plugin.toml').write_text(manifest)
        self.save('consumer-manifest.toml', manifest)
        self.api_call('plugin.link', {'path': str(consumer), 'enabled': True})
        def published():
            value = projection(self.endpoint)
            if any(owner == CONSUMER and fragment['name'] == 'command-palette'
                   for owner, fragment in exports(value)):
                return value
        self.initial_projection = until(published, 'actual ordinary consumer publication')
        self.save('initial-projection.json', self.initial_projection)
        self.catalog = self.api_call('ui.command.list')
        self.receipt['boot_id'] = self.catalog['boot_id']
        palette = next(fragment['tree'] for owner, fragment in exports(self.initial_projection)
                       if owner == CONSUMER and fragment['name'] == 'command-palette')
        root = palette['children']
        assert root[0]['type'] == 'row' and root[0]['context'] == self.context
        assert root[0]['menu']['type'] == 'column', root
        assert all(node['type'] == 'use' for node in root[1:]), root
        self.receipt['consumer_placement'] = palette

    def attach(self):
        if self.name == 'restart-idle-agents':
            assert self.home in self.endpoint.parents, 'only owned endpoints may be observed'
            self.tap = OwnedEndpointTap(self.endpoint, self.evidence)
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 110, 0, 0))
        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        try:
            self.native = subprocess.Popen([str(self.binary), '--session', 'interactions'],
                                           cwd=self.home, env=self.env, stdin=slave, stdout=slave,
                                           stderr=slave, preexec_fn=controlling_terminal, close_fds=True)
        finally:
            os.close(slave)
        def pump():
            while not self.stop.is_set():
                try:
                    ready, _, _ = select.select([self.master], [], [], 0.05)
                    if ready:
                        block = os.read(self.master, 65536)
                        if not block:
                            return
                        self.output.extend(block)
                except OSError:
                    return
        self.drain = threading.Thread(target=pump, daemon=True)
        self.drain.start()
        self.rendered('Review')
        assert self.native.poll() is None
        time.sleep(0.15)

    def rendered(self, text, start=0):
        def visible():
            assert self.native.poll() is None, ('actual native client exited', bytes(self.output))
            return text.encode() in self.output[start:]
        until(visible, 'native rendering: ' + text)

    def keys(self, data):
        assert self.native.poll() is None, 'native process already exited'
        self.receipt['events'].append({'native_bytes_hex': data.hex(), 'at': time.monotonic()})
        os.write(self.master, data)
        # PTY events remain separate native event batches, not a pasted string.
        time.sleep(0.08)

    def root(self):
        self.keys(b'\x1bOP')
        def root_visible():
            assert self.native.poll() is None, 'native client exited while opening root'
            lines = native_screen(bytes(self.output))
            # The submenu breadcrumb also contains Captured Target. Require the
            # actual root header and its selectable sibling row, not old bytes.
            return any(re.search(r'(?:^|\s)Commands(?:\s{2,}|$)', line) for line in lines) and any(
                'Captured Target' in line for line in lines)
        until(root_visible, 'native current screen shows command root')

    def branch(self):
        self.keys(b'Captured Target')
        self.keys(b'\r')

    def command(self, key, open_root=True, resource=True):
        if open_root:
            self.root()
            if resource:
                self.branch()
        def resolved():
            current = self.api_call('ui.command.list')
            found = [item for item in current['commands'] if item['key'] == key]
            assert len(found) == 1, (key, found)
            command = found[0]
            if command.get('diagnostic'):
                return None
            return command if any(fragment['name'] == command['fragment_name']
                                  for _, fragment in exports(projection(self.endpoint))) else None
        command = until(resolved, 'selected actual catalog locator becomes resolvable')
        self.receipt['selected_command'] = command
        self.keys(command['title'].encode())
        self.rendered(command['title'])
        self.keys(b'\r')
        return command

    def snapshot(self):
        # Keep full API replies in api.jsonl; comparisons use stable identities
        # and labels, not focus-dependent counts, numbering or agent status.
        workspaces = self.api_call('workspace.list')['workspaces']
        tabs = self.api_call('tab.list')['tabs']
        return {'workspaces': sorted(
                    ({key: item[key] for key in ('workspace_id', 'label')} for item in workspaces),
                    key=lambda item: item['workspace_id']),
                'tabs': sorted(
                    ({key: item[key] for key in ('tab_id', 'workspace_id', 'label')} for item in tabs),
                    key=lambda item: item['tab_id']),
                'focused_tab_id': self.api_call('session.snapshot')['snapshot']['focused_tab_id']}

    def focus(self, tab):
        self.api_call('tab.focus', {'tab_id': tab})
        until(lambda: self.snapshot()['focused_tab_id'] == tab, 'actual API focus change')
        time.sleep(0.2)  # Native client must observe this live visit before next input.

    def unchanged(self, baseline):
        time.sleep(0.25)
        now = self.snapshot()
        assert now == baseline, (baseline, now)
        return now

    def action_logs(self):
        return [item for item in self.api_call('plugin.log.list', {'plugin_id': OWNER, 'limit': 200})['logs']
                if item.get('action_id') == 'close-space']

    def no_leaks(self):
        time.sleep(0.15)
        observed = {pane: path.read_bytes().hex() for pane, path in self.sinks.items()}
        self.save('pane-input-witnesses.json', observed)
        assert not any(observed.values()), ('native input leaked into owned pane', observed)
        unexpected = self.evidence / 'wrong-owner-input.jsonl'
        assert not unexpected.exists() or not unexpected.read_bytes(), 'consumer or owner producer received an action'

    def blocked_confirmation(self, mode):
        if self.configuration['owner_mode'] != mode:
            self.configuration['owner_mode'] = mode
            self.config.write_text(json.dumps(self.configuration))
        def blocked():
            catalog = self.api_call('ui.command.list')
            item = next(command for command in catalog['commands'] if command['key'] == CLOSE_SPACE)
            value = projection(self.endpoint)
            # A config edit is not proof of publication. In particular the
            # second #22 phase must not accept the previous missing-export
            # diagnostic before the wrong-action tree actually reaches Core.
            ordinary = [fragment for owner, fragment in exports(value)
                        if owner == OWNER and fragment['name'] == (
                            'absent.confirmation' if mode == 'missing' else 'commands.confirm-space')]
            if mode == 'pending':
                published = any(ui['plugin_id'] == OWNER and ui.get('declaration') is not None
                                and not ui['declaration'].get('fragments') for ui in value['ui'])
            elif mode == 'ambiguous':
                published = len(ordinary) == 2
            else:
                published = len(ordinary) == 1
                if published and mode == 'wrong-action':
                    published = ordinary[0]['tree']['children'][1].get('action') == 'host:focus'
                elif published and mode == 'context':
                    published = ordinary[0]['tree']['children'][1].get('context') == self.other_context
            if not published:
                return None
            trees = [(owner, fragment) for owner, fragment in exports(value)
                     if fragment['name'] == item['fragment_name']]
            diagnostic = item.get('diagnostic')
            if diagnostic and not trees:
                return catalog, item, value
        catalog, item, value = until(blocked, 'unavailable confirmation retains diagnostic metadata without wrapper')
        assert OWNER in item['diagnostic'] and 'close-space' in item['diagnostic'], item
        self.save('blocked-' + mode + '-catalog.json', catalog)
        self.save('blocked-' + mode + '-projection.json', value)
        baseline, logs = self.snapshot(), self.action_logs()
        self.root()
        self.branch()
        self.keys(item['title'].encode())
        self.keys(b'\r')
        self.keys(b'\r')
        assert self.action_logs() == logs
        self.unchanged(baseline)
        self.receipt.setdefault('blocked_exports', []).append({'mode': mode, 'command': item})
        self.keys(b'\x1b')

    def finish(self):
        self.no_leaks()
        self.receipt['final_state'] = self.snapshot()
        self.save('final-projection.json', projection(self.endpoint))
        self.save('plugins.json', self.api_call('plugin.list'))
        self.save('action-logs.json', self.api_call('plugin.log.list', {'limit': 200}))
        self.save('receipt.json', self.receipt)
        return self.receipt

    def close(self):
        if self.api.exists() and self.core and self.core.poll() is None:
            for name, method in (('diagnostic-plugins.json', 'plugin.list'),
                                 ('diagnostic-plugin-logs.json', 'plugin.log.list')):
                try:
                    self.save(name, self.api_call(method))
                except (OSError, AssertionError) as error:
                    self.save(name, {'diagnostic_error': str(error)})
            try:
                self.save('diagnostic-endpoint-ui.json', projection(self.endpoint))
            except (OSError, AssertionError) as error:
                self.save('diagnostic-endpoint-ui.json', {'diagnostic_error': str(error)})
        if self.native is not None and self.native.poll() is None:
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
        self.save('pane-input-witnesses.json', {
            pane: path.read_bytes().hex() for pane, path in self.sinks.items()})
        if self.tap is not None:
            self.tap.close()
        if self.core is not None and self.core.poll() is None:
            try:
                self.api_call('server.stop')
                self.core.wait(timeout=5)
            except (OSError, AssertionError, subprocess.TimeoutExpired):
                self.core.terminate()
                try:
                    self.core.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.core.kill()
                    self.core.wait()
        self.stderr.close()
        self.temporary.cleanup()


def name_case(session, name, index):
    action, variation = name.rsplit('-', 1)
    key = core_key(action)
    session.install([key])
    session.attach()
    baseline = session.snapshot()
    session.command(key, resource=action != 'new-space')
    if action.startswith('rename') and variation == 'valid':
        session.focus(session.c)
        baseline['focused_tab_id'] = session.c
    value = f'C16_Name_{index}'
    if variation == 'blank':
        session.keys(b'   ')
        session.keys(b'\r')
        session.receipt['blank_result'] = session.unchanged(baseline)
        # Subsequent valid submit must still reach the captured name owner. This
        # positive control proves blank did not merely close the interaction.
        session.keys(b'\x7f\x7f\x7f')
    session.keys(value.encode())
    session.rendered(value)
    if variation == 'cancel':
        session.keys(b'\x1b')
        session.receipt['asserted_effect'] = session.unchanged(baseline)
        return
    session.keys(b'\r')
    def changed():
        now = session.snapshot()
        field = 'workspaces' if action.endswith('space') else 'tabs'
        rows = [item for item in now[field] if item['label'] == value]
        if not rows:
            return None
        assert len(rows) == 1, now
        row = rows[0]
        if action == 'new-space':
            assert len(now['workspaces']) == len(baseline['workspaces']) + 1
            assert len(now['tabs']) == len(baseline['tabs']) + 1
            assert [item for item in now['workspaces']
                    if item['workspace_id'] != row['workspace_id']] == baseline['workspaces']
            assert [item for item in now['tabs']
                    if item['workspace_id'] != row['workspace_id']] == baseline['tabs']
        elif action == 'new-tab':
            assert len(now['tabs']) == len(baseline['tabs']) + 1
            assert now['workspaces'] == baseline['workspaces']
            assert row['workspace_id'] == session.workspace
            assert [item for item in now['tabs'] if item['tab_id'] != row['tab_id']] == baseline['tabs']
        else:
            identity = 'workspace_id' if action == 'rename-space' else 'tab_id'
            target = session.workspace if action == 'rename-space' else session.b
            assert row[identity] == target
            assert len(now['workspaces']) == len(baseline['workspaces'])
            assert len(now['tabs']) == len(baseline['tabs'])
            untouched = [item for item in baseline[field] if item[identity] != target]
            assert [item for item in now[field] if item[identity] != target] == untouched
            other_field = 'tabs' if field == 'workspaces' else 'workspaces'
            assert now[other_field] == baseline[other_field]
        return now
    session.receipt['asserted_effect'] = until(changed, 'actual captured name mutation')


def close_case(session, name):
    plugin = name.startswith('close-space')
    key = CLOSE_SPACE if plugin else core_key('close-tab')
    session.install([key])
    session.attach()
    baseline, logs = session.snapshot(), session.action_logs()
    command = session.command(key)
    def wrapper():
        value = projection(session.endpoint)
        return next((fragment['tree'] for owner, fragment in exports(value)
                     if fragment['name'] == command['fragment_name']), None)
    tree = until(wrapper, 'actual generated confirmation wrapper')
    assert not tree.get('action') and tree['menu']['type'] == 'column', tree
    session.unchanged(baseline)  # Merely opening a confirmation never dispatches.
    session.keys(('Cancel' if name.endswith('cancel') else command['title']).encode())
    session.keys(b'\r')
    if name.endswith('cancel'):
        session.unchanged(baseline)
        assert session.action_logs() == logs
    else:
        field, identity, target = ('workspaces', 'workspace_id', session.workspace) if plugin else (
            'tabs', 'tab_id', session.b)
        def removed():
            now = session.snapshot()
            if any(item[identity] == target for item in now[field]):
                return None
            expected = [item for item in baseline[field] if item[identity] != target]
            assert now[field] == expected, (baseline, now)
            if plugin:
                assert {item['tab_id'] for item in now['tabs']} == {
                    item['tab_id'] for item in baseline['tabs'] if item['workspace_id'] != target}
            else:
                assert now['workspaces'] == baseline['workspaces']
            return now
        session.receipt['asserted_effect'] = until(removed, 'actual selected resource removal')
        if plugin:
            after = until(lambda: session.action_logs() if len(session.action_logs()) == len(logs) + 1 else None,
                          'one original compiled owner invocation')
            assert len(after) == len(logs) + 1, after


def run_case(session, name, index):
    session.start()
    if name.startswith(('new-space-', 'new-tab-', 'rename-space-', 'rename-tab-')):
        name_case(session, name, index)
    elif name.startswith(('close-tab-', 'close-space-')):
        close_case(session, name)
    elif name.startswith('confirmation-'):
        modes = {'confirmation-pending': 'pending', 'confirmation-missing-or-wrong': 'missing',
                 'confirmation-context': 'context', 'confirmation-ambiguous': 'ambiguous',
                 'confirmation-recovery': 'pending'}
        session.install([CLOSE_SPACE], owner_mode=modes[name])
        session.attach()
        session.blocked_confirmation(modes[name])
        if name == 'confirmation-missing-or-wrong':
            session.blocked_confirmation('wrong-action')
        if name == 'confirmation-recovery':
            session.configuration['owner_mode'] = 'valid'
            session.config.write_text(json.dumps(session.configuration))
            def recovered():
                item = next(item for item in session.api_call('ui.command.list')['commands']
                            if item['key'] == CLOSE_SPACE)
                return not item.get('diagnostic') and any(
                    owner == OWNER and fragment['name'] == item['fragment_name']
                    for owner, fragment in exports(projection(session.endpoint)))
            until(recovered, 'ordinary owner stream publication restores generated confirmation')
            baseline, logs = session.snapshot(), session.action_logs()
            command = session.command(CLOSE_SPACE)
            session.unchanged(baseline)
            session.keys(command['title'].encode())
            session.keys(b'\r')
            until(lambda: not any(item['workspace_id'] == session.workspace
                                 for item in session.snapshot()['workspaces']),
                  'recovered confirmation executes actual original compiled owner action')
            after = session.snapshot()
            assert after['workspaces'] == [item for item in baseline['workspaces']
                                           if item['workspace_id'] != session.workspace]
            until(lambda: len(session.action_logs()) == len(logs) + 1, 'only one close-space invocation')
            session.receipt['asserted_effect'] = after
    elif name in ('previous-empty', 'previous-global', 'rename-branch', 'new-native-scope'):
        selected = core_key('new') if name == 'new-native-scope' else core_key('focus', 'previous')
        rename = core_key('rename-tab')
        session.install([selected, rename])
        session.attach()
        if name == 'previous-global':
            session.focus(session.c)
            session.focus(session.a)
        if name == 'new-native-scope':
            session.focus(session.c)
        baseline = session.snapshot()
        session.root()
        session.branch()  # Menu-bearing B row is genuinely in native ancestry.
        if name == 'rename-branch':
            session.focus(session.c)
            baseline['focused_tab_id'] = session.c
            session.command(rename, open_root=False)
            session.keys(b'C16_B_only')
            session.keys(b'\r')
            until(lambda: any(item['tab_id'] == session.b and item['label'] == 'C16_B_only'
                              for item in session.snapshot()['tabs']), 'resource B rename after navigation to C')
            now = session.snapshot()
            assert [item for item in now['tabs'] if item['tab_id'] != session.b] == [
                item for item in baseline['tabs'] if item['tab_id'] != session.b]
            assert now['workspaces'] == baseline['workspaces']
        else:
            session.command(selected, resource=False)  # F1 resets resource ancestry.
            if name == 'previous-empty':
                session.unchanged(baseline)
            elif name == 'previous-global':
                until(lambda: session.snapshot()['focused_tab_id'] == session.c,
                      'native Previous chooses actual previous C, never popup B')
                now = session.snapshot()
                assert now['tabs'] == baseline['tabs'] and now['workspaces'] == baseline['workspaces']
            else:
                session.keys(b'C16_native_scope')
                session.keys(b'\r')
                def created():
                    now = session.snapshot()
                    rows = [item for item in now['tabs'] if item['label'] == 'C16_native_scope']
                    if not rows:
                        return None
                    assert len(rows) == 1 and rows[0]['workspace_id'] == session.other, now
                    assert len(now['tabs']) == len(baseline['tabs']) + 1
                    assert now['workspaces'] == baseline['workspaces']
                    return now
                until(created, 'host:new uses native current workspace, not captured Review')
        session.receipt['asserted_effect'] = session.snapshot()
    else:
        key = core_key('focus', 'targeted') if name == 'targeted-focus' else core_key(name)
        session.install([key])
        session.attach()
        baseline = session.snapshot()
        agents = session.api_call('agent.list') if name == 'restart-idle-agents' else None
        session.command(key, resource=name == 'targeted-focus')
        if name == 'detach':
            until(lambda: session.native.poll() is not None, 'only invoking native client detaches')
            assert session.native.returncode == 0, session.native.returncode
            assert session.core.poll() is None
            assert session.api_call('ui.command.list')['boot_id'] == session.receipt['boot_id']
            session.unchanged(baseline)
        elif name == 'targeted-focus':
            until(lambda: session.snapshot()['focused_tab_id'] == session.b, 'captured target really focused')
            now = session.snapshot()
            assert now['tabs'] == baseline['tabs'] and now['workspaces'] == baseline['workspaces']
        else:
            # The session has only owned /bin/sh byte-witness panes. Do not
            # manufacture eligible OMP agents or call restart via a second route.
            assert agents['agents'] == [], agents
            session.receipt['restart_result'] = until(
                session.tap.restart_result, 'one native restart request and its actual correlated empty Core result')
            assert session.api_call('agent.list') == agents
            session.unchanged(baseline)
        session.receipt['asserted_effect'] = session.snapshot()
    return session.finish()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('candidate', 'producer-root', 'output-dir'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--revision', required=True)
    args = parser.parse_args()
    args.candidate = args.candidate.resolve()
    args.producer_root = args.producer_root.resolve()
    args.output_dir = args.output_dir.resolve()
    if args.output_dir.exists() and any(args.output_dir.iterdir()):
        parser.error('--output-dir must be fresh or empty; existing proof receipts are never overwritten')
    args.output_dir.mkdir(parents=True, exist_ok=True)
    args.consumer = Path(__file__).with_name('command_interaction_consumer.py').resolve()
    args.candidate_hash = digest(args.candidate)
    root = args.producer_root / 'plugins' if (args.producer_root / 'plugins').is_dir() else args.producer_root
    args.producer_hashes = {item: digest(root / item) for item in PRODUCERS}
    args.consumer_hashes = {name: digest(args.consumer.with_name(name)) for name in (
        'command_interaction_consumer.py', 'command_interactions.py', 'command_compatibility.py',
        'command_definitions.py', 'neutral_consumer.py')}
    receipts = []
    assert len(EXAMPLES) == 28
    for index, (name, authored) in enumerate(EXAMPLES, 1):
        session = OwnedSession(args, index, name)
        try:
            receipt = run_case(session, name, index)
            receipt.update(authored_interaction=authored, status='passed')
            session.save('receipt.json', receipt)
        except Exception:
            receipt = {**session.receipt, 'authored_interaction': authored, 'status': 'failed',
                       'failure': traceback.format_exc()}
            session.save('receipt.json', receipt)
        finally:
            try:
                session.close()
            except Exception:
                receipt.update(status='failed', cleanup_failure=traceback.format_exc())
                session.save('receipt.json', receipt)
        receipts.append(receipt)
        (args.output_dir / 'receipts.json').write_text(json.dumps(receipts, indent=2))
    failed = [item['example'] for item in receipts if item['status'] != 'passed']
    print(json.dumps({'examples': len(receipts), 'failed_examples': failed,
                      'receipts': str(args.output_dir / 'receipts.json')}))
    return 1 if failed else 0


if __name__ == '__main__':
    raise SystemExit(main())
