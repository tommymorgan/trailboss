"""Platform-owned discovery producer/probe; no application or native-client imports."""
import json
import os
import socket
import struct
import sys
from neutral_consumer import Reader, read_exact, send


def request(path, method):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(8)
        connection.connect(path)
        connection.sendall((json.dumps({'id': 'catalog-consumer', 'method': method, 'params': {}}) + '\n').encode())
        data = bytearray()
        while b'\n' not in data:
            part = connection.recv(65536)
            assert part, 'Core closed JSON API without a response'
            data.extend(part)
        return json.loads(data.split(b'\n')[0])


def catalog(path):
    reply = request(path, 'ui.command.list')
    assert 'result' in reply, f'C01 requires the real ui.command.list API, received {reply}'
    result = reply['result']
    assert result['boot_id'] and result['commands'], result
    return result


def selected(commands, keys=None):
    if keys is None:
        keys = [{'kind': 'core', 'action_id': 'host:focus', 'mode': 'targeted'},
                {'kind': 'plugin', 'plugin_id': 'demo.owner', 'action_id': 'record-target'}]
    return [next(command for command in commands if command['key'] == key) for key in keys]


def producer(config):
    commands = selected(catalog(os.environ['HERDR_SOCKET_PATH'])['commands'], config.get('keys'))
    tree = {'type': 'column', 'children': [
        {'type': 'row', 'context': config['context'], 'children': [
            {'type': 'use', 'name': command['fragment_name']}]} for command in commands]}
    print(json.dumps({'herdr': 'widget_tree', 'version': 1,
                      'zones': [{'zone': 'list', 'tree': tree}]}), flush=True)
    for _ in sys.stdin:
        # This consumer has no actions. Any invocation is a wrong-owner defect.
        with open(config['consumer_invocations'], 'a', encoding='utf8') as output:
            output.write('WRONG OWNER\n')


def probe(config, api, endpoint, output):
    discovered = catalog(api)
    commands = selected(discovered['commands'], config.get('keys'))
    for command in commands:
        assert all(field in command for field in ('key', 'title', 'contexts', 'target', 'fragment_name')), command
        assert command['title'] and command['fragment_name'], command
    state = {'catalog': discovered, 'selections': [], 'declarations': []}
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(8)
        connection.connect(endpoint)
        send(connection, 20, 'endpoint.hello.v1', json.dumps({
            'generation': 1, 'cell_width_px': 0, 'cell_height_px': 0,
            'surface_size': {'cols': 80, 'rows': 24}, 'pixel_mouse': False,
            'direct_graphics': False, 'endpoint_keybindings': False, 'mouse_capture': False,
            'surface_active': False, 'surface_reuse': False, 'surface_delta': False,
            'surface_scroll': False, 'snapshot_codecs': ['shell.snapshot.v1'],
            'surface_codecs': ['shell.surface.v1'], 'input_codecs': ['shell.input.semantic.v1'],
            'blob_codecs': ['shell.blob.v1']}))
        boot = None
        while True:
            length = struct.unpack('<I', read_exact(connection, 4))[0]
            assert length <= 2 * 1024 * 1024, 'oversized generation-1 frame'
            reader = Reader(read_exact(connection, length))
            if reader.integer() != 20:
                continue
            kind, payload = reader.text(), reader.text()
            reader.finish()
            value = json.loads(payload)
            if kind == 'shell.snapshot.v1':
                boot = value['boot_id']
            elif kind == 'endpoint.plugin-ui.v1':
                state['declarations'] = value['ui']
            if not boot or not state['declarations']:
                continue
            exports = {}
            consumer = None
            for item in state['declarations']:
                declaration = item.get('declaration')
                if not declaration:
                    continue
                if item['plugin_id'] == 'demo.consumer':
                    consumer = declaration
                for fragment in declaration.get('fragments', []):
                    exports[fragment['name']] = (item['plugin_id'], fragment['tree'])
            found = []
            def visit(node, owner, context, locator=None):
                context = {**context, **{key: value for key, value in node.get('context', {}).items() if value is not None}}
                if node['type'] == 'use':
                    exported = exports.get(node['name'])
                    if exported:
                        visit(exported[1], exported[0], context, node['name'])
                    return
                if node.get('action'):
                    found.append({'plugin_id': owner, 'action_id': node['action'],
                                  'context': context, 'fragment_name': locator})
                for key in ('children', 'entries'):
                    for child in node.get(key, []):
                        visit(child, owner, context, locator)
            if consumer:
                for placement in consumer['zones']:
                    visit(placement['tree'], 'demo.consumer', {})
            if len(found) == len(commands):
                assert boot == discovered['boot_id'], 'catalog and actual projection belong to different boots'
                assert [item['fragment_name'] for item in found] == [item['fragment_name'] for item in commands]
                for selection, command in zip(found, commands):
                    key = command['key']
                    assert selection['action_id'] == key['action_id'], found
                    expected_owner = key['plugin_id'] if key['kind'] == 'plugin' else 'trailboss.core'
                    assert selection['plugin_id'] == expected_owner, found
                assert all(item['context'] == config['context'] for item in found), found
                state['selections'] = found
                with open(output, 'w', encoding='utf8') as artifact:
                    json.dump(state, artifact)
                return


if __name__ == '__main__':
    mode, config_path, *arguments = sys.argv[1:]
    with open(config_path, encoding='utf8') as source:
        config = json.load(source)
    if mode == 'produce':
        producer(config)
    else:
        probe(config, *arguments)
