"""Existing plugin-owned name interaction used by the palette handoff witness."""
import json
import os
from pathlib import Path
import socket
import sys


def request(endpoint, method, params):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(5)
        connection.connect(endpoint)
        connection.sendall((json.dumps({'id': 'owner-input', 'method': method, 'params': params}) + '\n').encode())
        with connection.makefile('r') as incoming:
            reply = json.loads(incoming.readline())
    if 'error' in reply:
        raise RuntimeError(reply['error'])
    return reply['result']


def record(path, **event):
    with Path(path).open('a') as output:
        output.write(json.dumps(event) + '\n')


def report_failure(error_type, error, traceback):
    trace = sys.argv[3] if sys.argv[1] == 'open' else os.environ.get('OWNER_INPUT_TRACE')
    if trace:
        record(trace, kind='failure', entrypoint=sys.argv[1],
               error_type=error_type.__name__, error=str(error))
    sys.__excepthook__(error_type, error, traceback)


sys.excepthook = report_failure

if sys.argv[1] == 'open':
    endpoint, trace = sys.argv[2:4]
    context = json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])
    record(trace, kind='invoke', owner=os.environ['HERDR_PLUGIN_ID'],
           action=os.environ['HERDR_PLUGIN_ACTION_ID'], context=context)
    request(endpoint, 'plugin.pane.open', {
        'plugin_id': os.environ['HERDR_PLUGIN_ID'], 'entrypoint': 'name',
        'workspace_id': context['workspace_id'], 'placement': 'tab', 'focus': True,
        'env': {'OWNER_INPUT_SOCKET': endpoint, 'OWNER_INPUT_TRACE': trace,
                'OWNER_INPUT_TARGET': context['tab_id']},
    })
elif sys.argv[1] == 'pane':
    endpoint = os.environ['OWNER_INPUT_SOCKET']
    trace = os.environ['OWNER_INPUT_TRACE']
    target = os.environ['OWNER_INPUT_TARGET']
    record(trace, kind='pane', owner=os.environ['HERDR_PLUGIN_ID'],
           entrypoint=os.environ['HERDR_PLUGIN_ENTRYPOINT_ID'],
           pane=os.environ['HERDR_PANE_ID'], target=target,
           context=json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON']))
    while True:
        print('OWNER NAME (type cancel to dismiss):', flush=True)
        value = sys.stdin.readline()
        if not value:
            break
        value = value.strip()
        if value == 'cancel':
            record(trace, kind='cancel', target=target)
            print('OWNER CANCELLED', flush=True)
            break
        if not value:
            record(trace, kind='reject', target=target)
            print('OWNER REJECTED EMPTY NAME', flush=True)
            continue
        result = request(endpoint, 'tab.rename', {'tab_id': target, 'label': value})
        record(trace, kind='submit', target=target, value=value, result=result)
        print('OWNER SAVED ' + value, flush=True)
        break
    # Keep the completed owner pane observable until the owned Core tears down.
    for _ in sys.stdin:
        pass
else:
    raise ValueError('unknown fixture entrypoint')
