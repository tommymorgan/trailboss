"""Independent real endpoint reader for the explicitly byte-equivalent test codec.

The original released consumer is never modified. This capable reader loads its
retained framing/Surface decoder, offers the new name, and verifies the actual
Core's selected name before decoding the real released-format Surface bytes.
"""
import importlib.util
import json
from pathlib import Path
import socket
import struct
import sys


def main():
    retained, endpoint, offered, expected, output = sys.argv[1:]
    spec = importlib.util.spec_from_file_location('retained_decoder', Path(retained) / 'neutral_consumer.py')
    decoder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(decoder)
    if offered not in ('shell.surface.v1', 'shell.surface.equivalent.v1'):
        raise ValueError('No independent decoder exists for this codec: ' + offered)
    hello = {'generation': 1, 'cell_width_px': 0, 'cell_height_px': 0,
             'surface_size': {'cols': 80, 'rows': 24}, 'pixel_mouse': False,
             'direct_graphics': False, 'endpoint_keybindings': False, 'mouse_capture': False,
             'surface_active': True, 'surface_reuse': False, 'surface_delta': False,
             'surface_scroll': False, 'snapshot_codecs': ['shell.snapshot.v1'],
             'surface_codecs': list(dict.fromkeys([offered, 'shell.surface.v1'])),
             'input_codecs': ['shell.input.semantic.v1'], 'blob_codecs': ['shell.blob.v1']}
    evidence = {'hello': hello, 'codec_semantics': 'byte-equivalent released PaneSurface payload'}
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(8)
        connection.connect(endpoint)
        decoder.send(connection, 20, 'endpoint.hello.v1', json.dumps(hello))
        while True:
            length = struct.unpack('<I', decoder.read_exact(connection, 4))[0]
            if length > 2 * 1024 * 1024:
                raise ValueError('Oversized actual Core frame')
            reader = decoder.Reader(decoder.read_exact(connection, length))
            tag = reader.integer()
            if tag == 20:
                kind, payload = reader.text(), reader.text()
                reader.finish()
                value = json.loads(payload)
                if kind == 'endpoint.welcome.v1':
                    if (value.get('error') or value['generation'] != 1 or
                            value['surface_codec'] != expected or
                            value['snapshot_codec'] != 'shell.snapshot.v1' or
                            value['input_codec'] != 'shell.input.semantic.v1' or
                            value['blob_codec'] != 'shell.blob.v1'):
                        raise ValueError('Actual Core did not negotiate the requested codec: ' + json.dumps(value))
                    evidence['welcome'] = value
                elif kind == 'shell.snapshot.v1':
                    evidence['snapshot'] = value
            elif tag == 13:
                surface = decoder.surface(reader)
                if 'neutral-live' in ''.join(cell['symbol'] for cell in surface['cells']):
                    evidence['decoded_marker'] = True
                    evidence['surface'] = surface
            elif tag == 8:
                enabled, sgr_pixels = reader.byte(), reader.byte()
                if enabled not in (0, 1) or sgr_pixels not in (0, 1):
                    raise ValueError('Invalid released MouseCapture booleans')
                reader.finish()
                evidence.setdefault('mouse_capture', []).append({
                    'enabled': bool(enabled), 'sgr_pixels': bool(sgr_pixels)})
            elif tag == 17:
                enabled = reader.byte()
                if enabled not in (0, 1):
                    raise ValueError('Invalid released keyboard-report boolean')
                reader.finish()
                evidence.setdefault('keyboard_report_all', []).append(bool(enabled))
            elif tag == 6:
                title = reader.option(reader.text)
                reader.finish()
                evidence.setdefault('window_titles', []).append(title)
            elif tag == 7:
                reader.finish()
                evidence['sound_reload_received'] = True
            elif tag == 16:
                flags, level = reader.integer(), reader.byte()
                if flags > 65535:
                    raise ValueError('Invalid released keyboard flags')
                reader.finish()
                evidence.setdefault('keyboard_protocol', []).append({'flags': flags, 'level': level})
            else:
                raise ValueError('Unexpected binary message on independent semantic connection: ' + str(tag))
            if all(key in evidence for key in ('welcome', 'snapshot', 'decoded_marker')):
                Path(output).write_text(json.dumps(evidence, indent=2) + '\n')
                return


if __name__ == '__main__':
    main()
