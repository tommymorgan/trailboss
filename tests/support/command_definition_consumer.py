"""Fixed C15 consumer: discover a compiled Core command, never register one."""
import json
from pathlib import Path
import sys

from command_compatibility import request


config = json.loads(Path(sys.argv[1]).read_text())
catalog = request(config['api'], 'ui.command.list')
key = {'kind': 'core', 'action_id': 'host:focus'}
command = next((item for item in catalog['commands'] if item['key'] == key), None)
fragment = command['fragment_name'] if command else config.get('stale_fragment')
children = [] if fragment is None else [{
    'type': 'row', 'context': config['context'],
    'children': [{'type': 'label', 'text': 'Captured Target', 'role': 'primary'}],
    'menu': {'type': 'column', 'children': [{'type': 'use', 'name': fragment}]},
}]
print(json.dumps({'herdr': 'widget_tree', 'version': 1, 'zones': [], 'fragments': [{
    'name': 'command-palette', 'priority': 1000000,
    'tree': {'type': 'column', 'children': children},
}]}), flush=True)
for line in sys.stdin:
    pass
