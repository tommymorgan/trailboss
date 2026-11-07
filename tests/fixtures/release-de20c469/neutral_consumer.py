"""Independent generation-1 consumer; see ADR 0005's wire contract.

No application/client imports. This intentionally supports plain text Surfaces,
not graphics, popups, delta/reuse, or a GUI renderer.
"""
import json
import socket
import struct
import sys


def uint(value):
    if value < 251:
        return bytes([value])
    for tag, fmt, maximum in [(251, '<H', 65535), (252, '<I', 4294967295), (253, '<Q', 18446744073709551615)]:
        if value <= maximum:
            return bytes([tag]) + struct.pack(fmt, value)
    raise ValueError('integer too large')


def string(value):
    encoded = value.encode()
    return uint(len(encoded)) + encoded


class Reader:
    def __init__(self, data):
        self.data = memoryview(data)
        self.offset = 0

    def take(self, length):
        end = self.offset + length
        if end > len(self.data):
            raise ValueError('truncated frame')
        result = self.data[self.offset:end]
        self.offset = end
        return result

    def byte(self):
        return self.take(1)[0]

    def integer(self):
        tag = self.byte()
        if tag < 251:
            return tag
        fmt = {251: '<H', 252: '<I', 253: '<Q'}[tag]
        return struct.unpack(fmt, self.take(struct.calcsize(fmt)))[0]

    def text(self):
        return bytes(self.take(self.integer())).decode()

    def option(self, decode):
        tag = self.byte()
        if tag not in (0, 1):
            raise ValueError('invalid option')
        return decode() if tag else None

    def rect(self):
        return [self.integer() for _ in range(4)]

    def finish(self):
        if self.offset != len(self.data):
            raise ValueError('unexpected trailing fields')


def read_exact(connection, size):
    chunks = bytearray()
    while len(chunks) < size:
        part = connection.recv(size - len(chunks))
        if not part:
            raise ValueError('Core closed stream')
        chunks.extend(part)
    return chunks


def send(connection, tag, *fields):
    data = uint(tag) + b''.join(string(field) for field in fields)
    connection.sendall(struct.pack('<I', len(data)) + data)


def surface(reader):
    boot = reader.text()
    projection, revision = reader.integer(), reader.integer()
    cells = []
    for _ in range(reader.integer()):
        cells.append({'symbol': reader.text(), 'fg': reader.integer(), 'bg': reader.integer(),
                      'modifier': reader.integer(), 'skip': reader.byte(),
                      'hyperlink': reader.option(reader.integer)})
    width, height = reader.integer(), reader.integer()
    cursor = reader.option(lambda: [reader.integer(), reader.integer(), reader.byte(), reader.byte()])
    hyperlinks = [reader.text() for _ in range(reader.integer())]
    graphics = list(reader.take(reader.integer()))
    panes = []
    for _ in range(reader.integer()):
        panes.append({'id': reader.text(), 'revision': reader.integer(),
                      'rect': reader.rect(), 'inner': reader.rect(),
                      'scrollbar': reader.option(reader.rect),
                      'scroll': reader.option(lambda: [reader.integer() for _ in range(3)]),
                      'focused': reader.byte(), 'mouse': reader.byte(), 'pixel_mouse': reader.byte(),
                      'alternate': reader.byte(), 'pixel_width': reader.integer(), 'pixel_height': reader.integer()})
    # This fixture has a single plain PTY: no splits, popup or graphics scene.
    for _ in range(5):
        if reader.integer() != 0:
            raise ValueError('unsupported split/popup/graphics fixture')
    reader.finish()
    if len(cells) != width * height:
        raise ValueError('Surface cell count')
    return {'boot': boot, 'projection': projection, 'revision': revision, 'cells': cells,
            'width': width, 'height': height, 'cursor': cursor, 'panes': panes,
            'hyperlinks': hyperlinks, 'graphics': graphics}


def main():
    path, target, output = sys.argv[1:]
    connection = socket.socket(socket.AF_UNIX)
    connection.settimeout(8)
    connection.connect(path)
    send(connection, 20, 'endpoint.hello.v1', json.dumps({
        'generation': 1, 'cell_width_px': 0, 'cell_height_px': 0,
        'surface_size': {'cols': 80, 'rows': 24}, 'pixel_mouse': False,
        'direct_graphics': False, 'endpoint_keybindings': False, 'mouse_capture': False,
        'surface_active': True, 'surface_reuse': False, 'surface_delta': False,
        'surface_scroll': False, 'snapshot_codecs': ['shell.snapshot.v1'],
        'surface_codecs': ['shell.surface.v1'], 'input_codecs': ['shell.input.semantic.v1'],
        'blob_codecs': ['shell.blob.v1']}))
    state = {'declarations': [], 'snapshots': [], 'surfaces': [], 'responses': [], 'declaration_history': []}
    action = None
    invoked = False
    while True:
        length = struct.unpack('<I', read_exact(connection, 4))[0]
        if length > 2 * 1024 * 1024:
            raise ValueError('oversized frame')
        reader = Reader(read_exact(connection, length))
        tag = reader.integer()
        if tag == 20:
            kind, data = reader.text(), reader.text()
            reader.finish()
            value = json.loads(data)
            if kind == 'endpoint.plugin-ui.v1':
                state['declarations'] = value['ui']
                state['declaration_history'].extend(value['ui'])
                def visit(node, plugin):
                    nonlocal action
                    if node.get('action') == 'focus-tab' and node.get('context', {}).get('tab_id') == target:
                        action = {'plugin_id': plugin, 'action_id': node['action'],
                                  'context': {**node['context'], 'invocation_source': 'plugin_ui'}}
                    for key in ('children', 'entries'):
                        for child in node.get(key, []):
                            visit(child, plugin)
                    for key in ('child', 'fallback', 'menu'):
                        if key in node:
                            visit(node[key], plugin)
                for item in value['ui']:
                    declaration = item.get('declaration')
                    if declaration:
                        for placement in declaration['zones']:
                            visit(placement['tree'], item['plugin_id'])
                        for fragment in declaration.get('fragments', []):
                            visit(fragment['tree'], item['plugin_id'])
                        if declaration.get('composition'):
                            visit(declaration['composition']['tree'], item['plugin_id'])
            elif kind == 'shell.snapshot.v1':
                state['snapshots'].append(value)
        elif tag == 13:
            state['surfaces'].append(surface(reader))
        elif tag == 18:
            boot, request = reader.text(), reader.text()
            final = reader.byte()
            data = bytes(reader.take(reader.integer())).decode()
            reader.finish()
            state['responses'].append({'boot': boot, 'id': request, 'final': final, 'data': data})
        elif tag in (1, 2):
            raise ValueError('terminal-renderer payload on semantic connection')
        ready = {item['plugin_id'] for item in state['declarations'] if item.get('declaration')}
        if action and state['snapshots'] and not invoked and {'tommymorgan.stock-ui', 'fixture.semantic-tone'} <= ready:
            state['action'] = action
            send(connection, 15, state['snapshots'][-1]['boot_id'], json.dumps({
                'id': 'widget-action-1', 'method': 'plugin.action.invoke', 'params': action}))
            invoked = True
        if invoked and state['snapshots'][-1].get('focused_tab_id') == target:
            for frame in state['surfaces']:
                text = ''.join(cell['symbol'] for cell in frame['cells'])
                if 'neutral-live' in text and frame['cursor']:
                    with open(output, 'w', encoding='utf8') as artifact:
                        json.dump(state, artifact, ensure_ascii=False)
                    connection.close()
                    return


if __name__ == '__main__':
    main()
