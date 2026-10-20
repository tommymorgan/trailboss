"""C16 ordinary UI producers. Never registers commands or handles an action.

The consumer reads the actual catalog, leaving labels and execution trees to Core.
The owner mode is only used for ordinary same-owner publication/error fixtures;
its manifest retains the shipped compiled sidebar's original action executable.
"""
import json
from pathlib import Path
import select
import sys
import time

from command_compatibility import request


def label(text):
    return {'type': 'label', 'text': text, 'role': 'primary'}


def column(children):
    return {'type': 'column', 'children': children}


def consumer(config):
    catalog = request(config['api'], 'ui.command.list')
    resources, global_commands = [], []
    for command in catalog['commands']:
        use = {'type': 'use', 'name': command['fragment_name']}
        key = command['key']
        if config.get('keys') and key not in config['keys']:
            continue
        client_local = key.get('action_id') == 'host:new' or command['target'] in (
            'client_local', 'global', 'native_scope')
        (global_commands if client_local else resources).append(use)
    children = [{'type': 'row', 'children': [label('Captured Target')],
                 'context': config['context'], 'menu': column(resources)}, *global_commands]
    Path(config['catalog_file']).write_text(json.dumps(catalog, indent=2))
    return {'herdr': 'widget_tree', 'version': 1, 'zones': [], 'fragments': [{
        'name': 'command-palette', 'priority': 1000000, 'tree': column(children)}]}


def owner(config):
    mode = config['owner_mode']
    fragments = []
    if mode != 'pending':
        affirmative = {'type': 'row', 'children': [label('Close space')],
                       'action': 'host:focus' if mode == 'wrong-action' else 'close-space'}
        if mode == 'context':
            affirmative['context'] = config['replacement_context']
        fragments = [{'name': 'absent.confirmation' if mode == 'missing' else 'commands.confirm-space',
                      'priority': config.get('priority', 0), 'tree': column([
                          {'type': 'row', 'children': [label('Cancel')], 'action': 'host:cancel'},
                          affirmative])}]
    return {'herdr': 'widget_tree', 'version': 1, 'zones': [], 'fragments': fragments}


def main():
    mode, config_path = sys.argv[1:]
    previous = None
    while True:
        config = json.loads(Path(config_path).read_text())
        declaration = consumer(config) if mode == 'consumer' else owner(config)
        encoded = json.dumps(declaration)
        if encoded != previous:
            print(encoded, flush=True)
            previous = encoded
        readable, _, _ = select.select([sys.stdin], [], [], 0.1)
        if readable:
            line = sys.stdin.readline()
            if not line:
                return
            # stream_actions is deliberately false. Receipt of any action is a
            # wrong-owner failure, never an alternate action implementation.
            with Path(config['unexpected_input']).open('a') as output:
                output.write(line)
        else:
            time.sleep(0.02)


if __name__ == '__main__':
    main()
