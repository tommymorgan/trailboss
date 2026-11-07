"""Manual retained-release compatibility check (standard-library only).

Reports are evidence, not release decisions. All runtime probes own their Core,
configuration root and PTYs. This command never installs or publishes anything.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from types import SimpleNamespace
import time
import tomllib

from command_packaged import Session


CODECS = ['shell.snapshot.v1', 'shell.surface.v1',
          'shell.input.semantic.v1', 'shell.blob.v1']
RETAINED = Path(__file__).resolve().parents[1] / 'fixtures/release-de20c469'


class ObservedProbeFailure(RuntimeError):
    def __init__(self, evidence):
        super().__init__('Retained independent consumer failed against the actual candidate Core')
        self.evidence = evidence


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def until(predicate, description, timeout=12):
    deadline = time.monotonic() + timeout
    while True:
        result = predicate()
        if result:
            return result
        if time.monotonic() >= deadline:
            raise RuntimeError('deadline: ' + description)
        time.sleep(0.02)


def baseline_object(path):
    try:
        value = json.loads(path.read_text())
    except json.JSONDecodeError as error:
        raise ValueError('Invalid baseline JSON ' + str(path) + ': ' + str(error)) from error
    if not isinstance(value, dict):
        raise ValueError('Invalid baseline JSON ' + str(path) + ': expected an object')
    return value


def baseline_identity(root, retained):
    inventory_path = retained / 'inventory.json'
    inventory = baseline_object(inventory_path)
    for key in ('source_revision', 'product', 'product_version', 'runtime_version',
                'protocol_version', 'endpoint_generation', 'binary_sha256',
                'deployment_manifest_sha256'):
        value = inventory.get(key)
        expected = int if key in ('protocol_version', 'endpoint_generation') else str
        if type(value) is not expected or not value or (expected is int and value < 1):
            raise ValueError('Invalid baseline inventory ' + str(inventory_path) + ': ' + key)
    if inventory['product'] != 'trailboss':
        raise ValueError('baseline product must be Trailboss, not an upstream substitute')
    required = {'neutral_consumer.py', 'api.schema.json', 'wire.rs',
                'widgets.rs', 'endpoint.rs', 'Cargo.toml'}
    if not isinstance(inventory.get('artifacts'), dict):
        raise ValueError('Invalid baseline inventory ' + str(inventory_path) + ': artifacts')
    missing = required - inventory.get('artifacts', {}).keys()
    if missing:
        raise ValueError('required retained baseline evidence missing: ' + ', '.join(sorted(missing)))
    manifest = baseline_object(root / 'build-manifest.json')
    if manifest.get('source') != inventory['source_revision'] or manifest.get('sha256') != inventory['binary_sha256']:
        raise ValueError('baseline source/product provenance does not match the identified retained build')
    if sha256(root / 'build-manifest.json') != inventory['deployment_manifest_sha256']:
        raise ValueError('baseline deployment manifest digest mismatch: build-manifest.json')
    if sha256(root / 'trailboss') != inventory['binary_sha256']:
        raise ValueError('baseline binary digest mismatch: trailboss')
    for item in manifest['assets']:
        if sha256(root / item['path']) != item['sha256']:
            raise ValueError('baseline artifact digest mismatch: ' + item['path'])
    for name, item in inventory['artifacts'].items():
        if not isinstance(item, dict) or any(not isinstance(item.get(key), str) or not item[key]
                for key in ('source_revision', 'source_path', 'sha256')):
            raise ValueError('Invalid baseline inventory ' + str(inventory_path) + ': artifacts.' + name)
        if sha256(retained / name) != item['sha256']:
            raise ValueError('retained source/consumer digest mismatch: ' + name)
    schema = baseline_object(retained / 'api.schema.json')
    if inventory['protocol_version'] != schema.get('protocol'):
        raise ValueError('Invalid baseline inventory ' + str(inventory_path) +
                         ': protocol_version differs from retained api.schema.json')
    tokens = rust_tokens((retained / 'endpoint.rs').read_text())
    for index, token in enumerate(tokens):
        if token == 'ENDPOINT_PROTOCOL_GENERATION' and tokens[index + 1:index + 4] == [':', 'u32', '=']:
            if inventory['endpoint_generation'] != int(tokens[index + 4]):
                raise ValueError('Invalid baseline inventory ' + str(inventory_path) +
                                 ': endpoint_generation differs from retained endpoint.rs')
            break
    else:
        raise ValueError('Retained endpoint.rs lacks its required endpoint generation definition')
    package = tomllib.loads((retained / 'Cargo.toml').read_text())['package']
    if package['name'] != inventory['product'] or package['version'] != inventory['product_version']:
        raise ValueError('baseline product/version differs from its pinned source Cargo.toml')
    return {**{key: value for key, value in inventory.items() if key != 'artifacts'},
            'codecs': CODECS, 'retained_artifacts': inventory['artifacts'],
            'producer_digests': {item['path']: item['sha256'] for item in manifest['assets']
                                 if item['path'].startswith('plugins/')}}


def shell_identity(path):
    pid = int(path.read_text().strip())
    # Linux start identity and PTY distinguish a preserved original shell from
    # a recreated process or numeric PID reuse.
    stat = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return {'pid': pid, 'start_ticks': stat[19],
            'pty': os.readlink(f'/proc/{pid}/fd/0')}


def runtime_probes(args):
    owned = Session(SimpleNamespace(client=args.candidate, producer_root=args.baseline,
                                    output_dir=args.output_dir), 'runtime')
    try:
        owned.start()
        created = owned.call('workspace.create', {
            'label': 'Compatibility', 'cwd': str(owned.home), 'focus': True})
        workspace = created['workspace']['workspace_id']
        pane = created['root_pane']['pane_id']
        for plugin in ('stock-ui', 'agent-tree', 'sidebar-menus'):
            owned.link(plugin)
        before = owned.home / 'before.pid'
        owned.call('pane.send_text', {'pane_id': pane, 'text':
            "compat_marker=retained-native-live; printf '%s\\n' \"$$\" > " + str(before) + "\n"})
        until(before.is_file, 'original marker shell')
        identity = shell_identity(before)
        # Reuse the native controlling-PTY harness, but attach the ORIGINAL
        # release executable to the distinct candidate Core.
        owned.args.client = args.baseline / 'trailboss'
        owned.attach()
        owned.args.client = args.candidate
        owned.keys(b"printf '\\033[2J\\033[H%s\\n' \"$compat_marker\"\r")
        owned.visible('retained-native-live')
        after = owned.home / 'after.pid'
        owned.keys(("printf '%s\\n' \"$$\" > " + str(after) + "\r").encode())
        until(after.is_file, 'retained native input reaches original shell')
        after_identity = shell_identity(after)
        assert identity == after_identity, ('original shell replaced', identity, after_identity)
        owned.keys(b'\x01')
        owned.keys(b'q')
        until(lambda: owned.native.poll() is not None, 'retained native detaches')
        assert owned.native.returncode == 0, ('retained native detach failed', owned.native.returncode)
        assert owned.core.poll() is None, 'native detach stopped candidate Core'
        assert shell_identity(after) == identity, 'detach replaced original shell'
        native = {'shell_identity_before': identity, 'shell_identity_after': after_identity,
                  'rendered_marker': True, 'input_receipt': after.read_text(),
                  'detached': True, 'core_alive_after_detach': True}
        second = owned.call('tab.create', {
            'workspace_id': workspace, 'label': 'Independent', 'cwd': str(owned.home), 'focus': False})
        target = second['tab']['tab_id']
        target_pane = second['root_pane']['pane_id']
        # The released independent decoder's documented plain-PTY fixture uses
        # this declaration. It is a real producer, not a server/client substitute.
        tone = owned.home / 'semantic-tone'
        tone.mkdir()
        (tone / 'trailboss-plugin.toml').write_text(
            'id = "fixture.semantic-tone"\nname = "Semantic tone"\nversion = "0.1.0"\n'
            'min_herdr_version = "0.9.0"\n[[ui]]\nid = "tone"\nsurface = "sidebar"\n'
            f'command = [{json.dumps(sys.executable)}, "producer.py"]\nstream = true\n')
        declaration = {'herdr': 'widget_tree', 'version': 1, 'zones': [
            {'zone': 'footer', 'tree': {'type': 'label', 'text': 'Neutral tone', 'role': 'muted'}}]}
        (tone / 'producer.py').write_text(
            f'import sys\nprint({json.dumps(json.dumps(declaration))}, flush=True)\n'
            'for line in sys.stdin:\n pass\n')
        owned.call('plugin.link', {'path': str(tone), 'enabled': True})
        owned.call('pane.send_text', {'pane_id': target_pane, 'text':
            "printf '\\033[2J\\033[Hneutral-%s\\033[4;7H\\033[6 q' live; read compatibility_hold\n"})
        output = owned.evidence / 'retained-consumer.json'
        consumer = subprocess.run([sys.executable, str(args.retained_contracts / 'neutral_consumer.py'),
                                   str(owned.endpoint), target, str(output)],
                                  env=owned.env, capture_output=True, text=True, timeout=20)
        if consumer.returncode != 0:
            current = owned.call('session.snapshot')['snapshot']
            raise ObservedProbeFailure({
                'consumer_exit_code': consumer.returncode, 'stderr': consumer.stderr,
                'expected_focused_tab_id': target,
                'observed_focused_tab_id': current.get('focused_tab_id'),
                'consumer_sha256': sha256(args.retained_contracts / 'neutral_consumer.py'),
                'native_probe': native})
        decoded = json.loads(output.read_text())
        assert decoded['snapshots'][-1]['focused_tab_id'] == target
        assert any('neutral-live' in ''.join(cell['symbol'] for cell in frame['cells'])
                   for frame in decoded['surfaces'])
        assert decoded['action']['action_id'] == 'focus-tab'
        extra_probes = []
        paired = {'target_tab_id': target}
        readers = [('retained', 'shell.surface.v1')]
        if args.probe_surface_codec:
            readers.append(('capable', args.probe_surface_codec))
        for name, offered in readers:
            receipt = owned.evidence / (name + '-negotiation.json')
            result = subprocess.run([
                sys.executable, str(Path(__file__).with_name('release_capability_consumer.py')),
                str(args.retained_contracts), str(owned.endpoint), offered, offered, str(receipt)],
                env=owned.env, capture_output=True, text=True, timeout=15)
            if result.returncode != 0:
                raise RuntimeError('Actual ' + name + ' codec negotiation failed: ' + result.stderr)
            paired[name] = json.loads(receipt.read_text())
            assert paired[name]['snapshot']['focused_tab_id'] == target
        extra_probes.append({'name': 'negotiated-surface-codec' if args.probe_surface_codec
                             else 'endpoint-negotiation', 'status': 'verified', 'evidence': paired})
        snapshot = owned.call('session.snapshot')
        runtime = owned.call('ping')
        return [
            {'name': 'retained-native', 'status': 'verified', 'evidence': native},
            {'name': 'retained-api', 'status': 'verified', 'evidence': {
                'methods': ['session.snapshot', 'workspace.create', 'pane.send_text', 'tab.create'],
                'response': snapshot, 'runtime_version': runtime['version']}},
            {'name': 'retained-plugin-action', 'status': 'verified', 'evidence': {
                'action': decoded['action'], 'focused_tab_id': target,
                'producer_root': str(args.baseline)}},
            {'name': 'retained-independent-endpoint', 'status': 'verified', 'evidence': {
                'consumer_sha256': sha256(args.retained_contracts / 'neutral_consumer.py'), 'codecs': CODECS,
                'generation': 1, 'artifact': str(output),
                'observed_widget_nodes': sorted(received_widget_nodes(decoded['declarations'])),
                'scope': 'plain PTY; no graphics, popup, delta, reuse or scroll encodings'}}] + extra_probes
    finally:
        owned.close()


def resolve_schema(root, schema):
    visited = set()
    while '$ref' in schema:
        reference = schema['$ref']
        if not reference.startswith('#/') or reference in visited:
            raise ValueError('unsupported or cyclic schema reference: ' + reference)
        visited.add(reference)
        schema = root
        for part in reference[2:].split('/'):
            schema = schema[part.replace('~1', '/').replace('~0', '~')]
    return schema


def required_field_changes(old_root, new_root, old, new, path='params', seen=None, accounted=None):
    seen = set() if seen is None else seen
    old = resolve_schema(old_root, old)
    new = resolve_schema(new_root, new)
    identity = (id(old), id(new))
    if identity in seen:
        return []
    seen.add(identity)
    additions = set(new.get('required', [])) - set(old.get('required', []))
    if accounted is not None and additions:
        accounted.setdefault(identity, set()).update(additions)
    changes = [path + '.' + field for field in sorted(additions)]
    for field in old.get('properties', {}).keys() & new.get('properties', {}).keys():
        changes.extend(required_field_changes(old_root, new_root,
            old['properties'][field], new['properties'][field], path + '.' + field, seen, accounted))
    if isinstance(old.get('items'), dict) and isinstance(new.get('items'), dict):
        changes.extend(required_field_changes(old_root, new_root,
            old['items'], new['items'], path + '[]', seen, accounted))
    return changes


def unclassified_schema_change(old_root, new_root, old, new, seen=None, accounted=None):
    """Conservative structural guard, not a JSON Schema compatibility validator."""
    seen = set() if seen is None else seen
    if isinstance(old, dict) and isinstance(new, dict):
        identity = (id(old), id(new))
        if identity in seen:
            return False
        seen = seen | {identity}
        old_target, new_target = resolve_schema(old_root, old), resolve_schema(new_root, new)
        classified = (accounted or {}).get((id(old_target), id(new_target)), set())
        old = {**old_target, **{k: v for k, v in old.items() if k != '$ref'}}
        new = {**new_target, **{k: v for k, v in new.items() if k != '$ref'}}
        ignored = {'title', 'description', 'examples', '$comment', '$defs', 'definitions'}
        for key in (old.keys() | new.keys()) - ignored:
            if key == 'required':
                # Suppress only requirements the explicit classifier actually visited.
                additions = set(new.get(key, [])) - set(old.get(key, []))
                if additions - classified or set(old.get(key, [])) - set(new.get(key, [])):
                    return True
                continue
            if key == 'properties':
                before, after = old.get(key, {}), new.get(key, {})
                for field in before.keys() | after.keys():
                    if field not in before and field in classified:
                        continue
                    if field not in before or field not in after or unclassified_schema_change(
                            old_root, new_root, before[field], after[field], seen, accounted):
                        return True
                continue
            if key not in old or key not in new:
                return True
            before, after = old[key], new[key]
            if key in ('oneOf', 'anyOf', 'enum', 'type') and isinstance(before, list) and isinstance(after, list):
                before = sorted(before, key=lambda value: json.dumps(value, sort_keys=True))
                after = sorted(after, key=lambda value: json.dumps(value, sort_keys=True))
            if key == 'default':
                if before != after:
                    return True
            elif unclassified_schema_change(old_root, new_root, before, after, seen, accounted):
                return True
        return False
    if isinstance(old, list) and isinstance(new, list):
        return len(old) != len(new) or any(unclassified_schema_change(
            old_root, new_root, before, after, seen, accounted) for before, after in zip(old, new))
    return old != new


def schema_findings(baseline, candidate, scope):
    old = {item['properties']['method']['const']: item
           for item in baseline['schemas']['request']['oneOf']}
    new = {item['properties']['method']['const']: item
           for item in candidate['schemas']['request']['oneOf']}
    findings = [{'contract': 'session-api', 'change': 'removed method', 'method': method,
                 'impact': 'breaking-contract', 'evidence_scope': scope,
                 'reason': 'A released request method is absent from the compared candidate schema.'}
                for method in sorted(old.keys() - new.keys())]
    accounted = {}
    for method in sorted(old.keys() & new.keys()):
        for field in required_field_changes(baseline, candidate,
                old[method], new[method], path='request', accounted=accounted):
            findings.append({'contract': 'session-api', 'change': 'new required request field',
                'method': method, 'field': field, 'impact': 'breaking-contract',
                'evidence_scope': scope,
                'reason': 'Released callers do not supply this newly required request field.'})
    findings.extend({'contract': 'session-api', 'change': 'new request method',
                     'method': method, 'impact': 'compatible-addition', 'evidence_scope': scope,
                     'reason': 'A new request method is additive to released callers. '
                               'Schema evidence does not establish its runtime availability or behavior.'}
                    for method in sorted(new.keys() - old.keys()))
    unknown = []
    for method in sorted(old.keys() & new.keys()):
        if unclassified_schema_change(baseline, candidate,
                old[method], new[method], accounted=accounted):
            unknown.append('request:' + method)
    for name in baseline['schemas'].keys() | candidate['schemas'].keys():
        before, after = baseline['schemas'].get(name), candidate['schemas'].get(name)
        if name == 'request':
            before = {key: value for key, value in before.items() if key != 'oneOf'}
            after = {key: value for key, value in after.items() if key != 'oneOf'}
        if before is None or after is None or unclassified_schema_change(
                baseline, candidate, before, after, accounted=accounted):
            unknown.append(name)
    if unknown:
        findings.append({'contract': 'session-api', 'change': 'unclassified public schema difference',
                         'impact': 'unverified-change', 'components': sorted(unknown),
                         'evidence_scope': scope,
                         'reason': 'These public schema differences are not covered by the checked '
                                   'addition/break rules. Compatibility and version impact remain unverified.'})
    return findings


def rust_tokens(source):
    tokens = re.findall(
        r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|[A-Za-z_]\w*|[0-9]+|[^\s]',
        source)
    return [token for token in tokens if not token.startswith(('//', '/*'))]


def serialized_declarations(source, kinds=('struct', 'enum')):
    # Keep ordered fields and serde attributes, discard formatting/comments.
    # This is deliberately a narrow declaration scanner, not a Rust evaluator.
    # Source evidence is reported separately from observed binary behavior.
    tokens = rust_tokens(source)
    declarations = {}
    for index in range(len(tokens) - 3):
        if tokens[index] != 'pub' or tokens[index + 1] not in kinds:
            continue
        name, opening = tokens[index + 2:index + 4]
        if opening not in ('{', '('):
            continue
        before = index - 1
        attributes = []
        serde_attributes = []
        while before >= 0 and tokens[before] == ']':
            end = before + 1
            depth = 1
            before -= 1
            while before >= 0 and depth:
                depth += (tokens[before] == ']') - (tokens[before] == '[')
                before -= 1
            if before < 0 or tokens[before] != '#':
                break
            attributes[0:0] = tokens[before:end]
            if 'serde' in tokens[before:end]:
                serde_attributes[0:0] = tokens[before:end]
            before -= 1
        if 'Serialize' not in attributes or 'Deserialize' not in attributes:
            continue
        depth, end = 1, index + 4
        closing = '}' if opening == '{' else ')'
        while end < len(tokens) and depth:
            depth += (tokens[end] == opening) - (tokens[end] == closing)
            end += 1
        if depth:
            raise ValueError('unclosed serialized declaration: ' + name)
        declarations[name] = {'kind': tokens[index + 1],
                              'signature': serde_attributes + tokens[index + 3:end]}
    return declarations


def binary_findings(old_source, new_source):
    old = serialized_declarations(old_source)
    new = serialized_declarations(new_source)
    findings = []
    for name in sorted(old):
        if new.get(name) == old[name]:
            continue
        enum = old[name]['kind'] == 'enum'
        findings.append({'contract': 'binary-layout',
            'change': 'enum variants changed' if enum else 'ordered binary payload changed',
            'type': name, 'impact': 'potential-breaking' if enum else 'breaking-contract',
            'evidence_scope': 'candidate source artifacts',
            'reason': ('Changed variant indices/vocabulary cannot be decoded by released readers '
                       'if emitted on their existing path; source comparison does not prove emission.')
                      if enum else
                      'Released ordered serialized fields differ; old readers require the original layout.'})
    return findings


def widget_findings(old_source, new_source):
    old = serialized_declarations(old_source)
    new = serialized_declarations(new_source)
    return [{'contract': 'widget-vocabulary', 'change': 'typed widget contract changed',
             'type': name, 'impact': 'potential-breaking',
             'evidence_scope': 'candidate source artifacts',
             'reason': 'An older typed widget reader may reject this vocabulary/shape if emitted; '
                       'source comparison does not establish actual emission or negotiation.'}
            for name in sorted(old) if new.get(name) != old[name]]


def named_endpoint_contracts(source):
    tokens = rust_tokens(source)
    names = set()
    for index in range(len(tokens) - 8):
        if tokens[index:index + 2] != ['pub', 'const']:
            continue
        identifier = tokens[index + 2]
        if 'CODEC' not in identifier and not identifier.endswith('_CAPABILITY'):
            continue
        if tokens[index + 3:index + 7] != [':', '&', 'str', '=']:
            continue
        literal = tokens[index + 7]
        if literal.startswith('"') and tokens[index + 8] == ';':
            names.add(('named-endpoint-codec' if 'CODEC' in identifier else 'endpoint-capability',
                       json.loads(literal)))
    return names


def endpoint_findings(old_source, new_source):
    old = named_endpoint_contracts(old_source)
    new = named_endpoint_contracts(new_source)
    findings = [{'contract': family, 'name': name, 'change': 'released endpoint name removed',
                 'impact': 'potential-breaking', 'evidence_scope': 'candidate source artifacts',
                 'reason': 'A released endpoint name is absent; source evidence alone does not establish retirement behavior.'}
                for family, name in sorted(old - new)]
    findings.extend({'contract': family, 'name': name, 'change': 'optional endpoint name added',
                     'impact': 'compatible-addition', 'evidence_scope': 'candidate source artifacts',
                     'reason': 'A named extension is declared while the released names remain. '
                               'Its actual availability and negotiation require runtime evidence.'}
                    for family, name in sorted(new - old) if old <= new)
    return findings


def compare_contracts(args, report):
    exported = subprocess.run([str(args.candidate), 'api', 'schema', '--json'],
                              capture_output=True, text=True, timeout=15, check=True)
    candidate_schema = json.loads(exported.stdout)
    report['candidate']['protocol_version'] = candidate_schema['protocol']
    report['candidate']['schema_sha256'] = hashlib.sha256(exported.stdout.encode()).hexdigest()
    retained_schema = json.loads((args.retained_contracts / 'api.schema.json').read_text())
    findings = schema_findings(retained_schema, candidate_schema, 'candidate binary schema export')
    if args.candidate_contracts:
        source_schema = json.loads((args.candidate_contracts / 'api.schema.json').read_text())
        findings.extend(schema_findings(retained_schema, source_schema, 'candidate source artifacts'))
        findings.extend(binary_findings((args.retained_contracts / 'wire.rs').read_text(),
                                       (args.candidate_contracts / 'wire.rs').read_text()))
        findings.extend(widget_findings((args.retained_contracts / 'widgets.rs').read_text(),
                                       (args.candidate_contracts / 'widgets.rs').read_text()))
        findings.extend(endpoint_findings((args.retained_contracts / 'endpoint.rs').read_text(),
                                         (args.candidate_contracts / 'endpoint.rs').read_text()))
        cargo = args.candidate_contracts / 'Cargo.toml'
        if cargo.is_file():
            report['candidate']['source_product_version'] = tomllib.loads(cargo.read_text())['package']['version']
        tokens = rust_tokens((args.candidate_contracts / 'wire.rs').read_text())
        for index, token in enumerate(tokens):
            if token == 'PROTOCOL_VERSION' and tokens[index + 1:index + 4] == [':', 'u32', '=']:
                report['candidate']['source_protocol_version'] = int(tokens[index + 4])
                break
    return findings


def received_widget_nodes(value):
    nodes = set()
    if isinstance(value, dict):
        if isinstance(value.get('type'), str):
            nodes.add(value['type'])
        for child in value.values():
            nodes.update(received_widget_nodes(child))
    elif isinstance(value, list):
        for child in value:
            nodes.update(received_widget_nodes(child))
    return nodes


def coverage_report(args, report):
    probes = {item['name']: item for item in report['probes'] if item['status'] == 'verified'}
    endpoint = probes.get('retained-independent-endpoint', {}).get('evidence', {})
    negotiation = probes.get('endpoint-negotiation', probes.get('negotiated-surface-codec', {}))
    welcome = negotiation.get('evidence', {}).get('retained', {}).get('welcome', {})
    methods = set()
    log = args.output_dir / 'runtime/api.jsonl'
    if log.is_file():
        methods.update(json.loads(line)['method'] for line in log.read_text().splitlines())
    if 'retained-plugin-action' in probes:
        methods.add('plugin.action.invoke')
    structural = ('source-artifacts-compared' if args.candidate_contracts else
                  'identical-artifact' if report['baseline']['binary_sha256'] ==
                  report['candidate']['binary_sha256'] else 'unverified')
    unverified = {name: 'not-exercised' for name in
                  ('graphics', 'popup', 'surface-delta', 'surface-reuse', 'surface-scroll')}
    if any(item['impact'] == 'unverified-change' for item in report['findings']):
        unverified['session-api'] = 'unclassified-schema-change'
    return {'overall': 'bounded-evidence', 'direction': report['direction'],
            'accepted_endpoint_generation': welcome.get('generation'),
            'methods': sorted(methods), 'codecs': [
                welcome[key] for key in ('snapshot_codec', 'surface_codec', 'input_codec', 'blob_codec')
                if key in welcome],
            'observed_widget_nodes': endpoint.get('observed_widget_nodes', []),
            'structural_contracts': structural,
            'source_binary_correspondence': report['candidate'].get(
                'source_binary_correspondence', 'unverified: caller-declared revision'),
            'unverified': unverified}


def version_impact(report):
    structural = report['coverage']['structural_contracts'] != 'unverified'
    unchanged = report['status'] == 'checked' and not report['findings'] and structural
    addition = (report['status'] == 'checked' and bool(report['findings']) and
                all(item['impact'] == 'compatible-addition' for item in report['findings']))
    breaking = any(item['impact'] in ('breaking-contract', 'potential-breaking', 'observed-incompatibility')
                   for item in report['findings'])
    unclassified = any(item['impact'] == 'unverified-change' for item in report['findings'])
    return {
        'stable_suggestion': 'major' if breaking else 'minor' if addition else 'conditional-patch' if unchanged else None,
        'reason_code': ('stable-public-contract-break' if breaking else
                        'backward-compatible-public-addition' if addition else
                        'no-detected-public-contract-change' if unchanged else
                        'unclassified-public-schema-change' if unclassified else
                        'insufficient-public-contract-evidence' if not structural else 'incomplete-impact-analysis'),
        'affected_contracts': sorted({item['contract'] for item in report['findings']}),
        'scope': 'checked-contracts', 'claims_complete_compatibility': False,
        'selected_release_version': None, 'provisional': True,
        'reason': ('No public contract change was detected in the compared artifacts and completed probes. '
                   'A patch may suffice if the remaining changes are compatible fixes.')
                  if unchanged else
                  'A compatible named public extension suggests minor for a stable contract, '
                  'provided the released path is preserved and the addition is negotiated as declared.'
                  if addition else
                  'An incompatible stable public contract implies major. Source-only differences are '
                  'provisional risks: they do not attest the candidate binary or prove a new variant was emitted.'
                  if breaking else 'The available comparison does not establish a version recommendation.',
        'limitations': ['Finite probes cannot classify unrelated changes or prove every semantic contract.',
                        'Plain-PTY evidence does not verify graphics, popups or optional Surface codecs.'],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', required=True, type=Path)
    parser.add_argument('--retained-contracts', type=Path, default=RETAINED,
                        help='Identified retained source/consumer inventory (never regenerated)')
    parser.add_argument('--candidate', required=True, type=Path)
    parser.add_argument('--candidate-revision', required=True)
    source_inputs = parser.add_mutually_exclusive_group()
    source_inputs.add_argument('--candidate-contracts', type=Path,
                        help='Explicit source contract artifacts; not an attestation of binary behavior')
    source_inputs.add_argument('--candidate-source-root', type=Path,
                        help='Compare source contracts from an existing candidate checkout without editing it')
    parser.add_argument('--stable-contract-reference',
                        help='Hypothetical stable SemVer reference for impact analysis, not artifact identity')
    parser.add_argument('--probe-surface-codec', choices=['shell.surface.equivalent.v1'],
                        help='Exercise a byte-equivalent named Surface codec with a capable independent reader')
    parser.add_argument('--output-dir', required=True, type=Path)
    args = parser.parse_args()
    args.baseline = args.baseline.resolve()
    args.candidate = args.candidate.resolve()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = {'direction': 'retained-client -> candidate-Core', 'probes': [], 'findings': []}
    if args.stable_contract_reference:
        report['version_reference'] = {
            'version': args.stable_contract_reference,
            'kind': 'hypothetical-stable-contract-reference',
            'role': 'hypothetical stable public-contract reference, not the retained product version'}
    phase = 'baseline-evidence'
    try:
        report['baseline'] = baseline_identity(args.baseline, args.retained_contracts)
        phase = 'candidate-evidence'
        if args.candidate_source_root:
            source = args.candidate_source_root.resolve()
            bundle = args.output_dir / 'candidate-contracts'
            bundle.mkdir(exist_ok=True)
            for name in ('api.schema.json', 'wire.rs', 'widgets.rs', 'endpoint.rs', 'Cargo.toml'):
                original = source / report['baseline']['retained_artifacts'][name]['source_path']
                shutil.copy2(original, bundle / name)
            args.candidate_contracts = bundle
        candidate_digest = sha256(args.candidate)
        report['candidate'] = {'source_revision': args.candidate_revision,
                               'binary_sha256': candidate_digest,
                               'product_version': (report['baseline']['product_version']
                                   if candidate_digest == report['baseline']['binary_sha256'] else None),
                               'product_version_evidence': 'identical binary' if
                                   candidate_digest == report['baseline']['binary_sha256'] else 'unverified'}
        if args.candidate_contracts:
            report['candidate']['contract_artifact_digests'] = {
                name: sha256(args.candidate_contracts / name)
                for name in ('api.schema.json', 'wire.rs', 'widgets.rs', 'endpoint.rs')}
            report['candidate']['source_binary_correspondence'] = 'unverified: explicit local source artifacts'
        report['findings'] = compare_contracts(args, report)
        phase = 'candidate-runtime'
        report['probes'] = runtime_probes(args)
        complete = all(probe['status'] == 'verified' for probe in report['probes'])
        incompatible = any(item['impact'] != 'compatible-addition' for item in report['findings'])
        report['status'] = ('contract-difference' if incompatible else
                            'checked' if complete else 'incomplete')
    except ObservedProbeFailure as error:
        report['findings'].append({
            'contract': 'action-behavior', 'change': 'retained action-driven consumer failed',
            'probe': 'retained-independent-endpoint', 'impact': 'observed-incompatibility',
            'evidence_scope': 'observed candidate Core', 'evidence': error.evidence,
            'reason': 'The original consumer did not complete its released action/focus/Surface path; '
                      'the recorded stderr and observed focus identify the failed probe, not a guessed cause.'})
        report['probes'].append({'name': 'retained-independent-endpoint',
                                 'status': 'failed', 'evidence': error.evidence})
        report.update(status='failed', error=str(error))
    except (OSError, ValueError, KeyError, TypeError, AssertionError, RuntimeError,
            subprocess.SubprocessError) as error:
        report.update(status='failed', error=str(error), error_contract=phase)
    if 'baseline' in report and 'candidate' in report:
        report['coverage'] = coverage_report(args, report)
        for probe in report['probes']:
            if probe['name'] == 'retained-api' and probe['status'] == 'verified':
                report['candidate']['runtime_version'] = probe['evidence']['runtime_version']
        report['version_impact'] = version_impact(report)
        versions = [report['baseline'].get('product_version'), report['candidate'].get('product_version')]
        pre_one = any(version and version.split('.')[0] == '0' for version in versions)
        report['version_impact']['pre_one'] = {
            'status': 'owner-policy-required' if pre_one else 'not-applicable',
            'selected_policy': None, 'suggested_product_bump': None,
            'stable_contract_implication': report['version_impact']['stable_suggestion'],
            'reason': 'For a 0.x product, the actual bump depends on Tommy’s chosen pre-1.0 policy. '
                      'Stable-contract implications are not a selected 0.x policy or release version.'
                      if pre_one else 'No known compared product version is below 1.0; '
                                      'unknown product identity is not inferred from source declarations.',
        }
        public_break = report['version_impact']['stable_suggestion'] == 'major'
        if public_break:
            report['public_path_obligation'] = {
                'version_bump_restores_v1': False,
                'requires_preserved_v1_or_negotiated_named_codec': True,
                'affected_readers': ['retained-generation-1-client'],
                'explanation': 'A package or private protocol bump neither repairs the original generation-1 '
                               'payload nor negotiates a replacement codec. Preserve the released path or '
                               'negotiate a new named codec while retaining V1. Source declarations and '
                               'actual binary observations remain separate evidence.',
            }
    (args.output_dir / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    return 0 if report['status'] == 'checked' else 1


if __name__ == '__main__':
    sys.exit(main())
