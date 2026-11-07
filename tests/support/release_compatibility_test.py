"""Public manual-command proofs; real artifacts are supplied by the developer.

Run from the repository root with TRAILBOSS_COMPAT_CANDIDATE naming a built
candidate. No fixture server, codec import, or historical artifact regeneration.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path(os.environ['TRAILBOSS_COMPAT_BASELINE']).resolve()


class ReleaseCompatibility(unittest.TestCase):
    def test_retained_release_client_works_with_candidate_core(self):
        candidate = Path(os.environ['TRAILBOSS_COMPAT_CANDIDATE']).resolve()
        with tempfile.TemporaryDirectory(prefix='release-command-proof-') as temporary:
            evidence = Path(temporary)
            result = subprocess.run([
                sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                '--baseline', str(BASELINE), '--candidate', str(candidate),
                '--candidate-revision', os.environ['TRAILBOSS_COMPAT_REVISION'],
                '--output-dir', str(evidence),
            ], cwd=ROOT, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            report = json.loads((evidence / 'report.json').read_text())
            self.assertEqual(report['baseline']['source_revision'],
                             'de20c4694668d6305e6ff650c47cfa3614abd5d0')
            self.assertEqual(report['baseline']['binary_sha256'],
                             '718444cbe83522f436c1872fbbf4efa055dae67533aeaa5a7d688618c4f83502')
            self.assertEqual(report['direction'], 'retained-client -> candidate-Core')
            self.assertEqual(report['candidate']['source_revision'],
                             os.environ['TRAILBOSS_COMPAT_REVISION'])
            self.assertEqual(report['baseline']['product_version'], '0.1.0')
            self.assertEqual(report['baseline']['protocol_version'], 22)
            self.assertEqual(report['baseline']['endpoint_generation'], 1)
            self.assertEqual(report['baseline']['codecs'], [
                'shell.snapshot.v1', 'shell.surface.v1',
                'shell.input.semantic.v1', 'shell.blob.v1'])
            probes = {item['name']: item for item in report['probes']}
            for name in ('retained-native', 'retained-api', 'retained-plugin-action',
                         'retained-independent-endpoint'):
                self.assertEqual(probes[name]['status'], 'verified', probes[name])
            native = probes['retained-native']['evidence']
            self.assertEqual(native['shell_identity_before'], native['shell_identity_after'])
            self.assertTrue(native['rendered_marker'])
            self.assertTrue(native['input_receipt'])
            self.assertTrue(native['detached'])
            self.assertTrue(native['core_alive_after_detach'])
            self.assertIn('shell.surface.v1', report['baseline']['codecs'])

    def test_regeneration_cannot_erase_released_contract_differences(self):
        selected = os.environ.get('TRAILBOSS_COMPAT_EXAMPLE')
        examples = [selected] if selected else [
            'removed-method', 'required-field', 'method-envelope', 'binary-field-order',
            'appended-enum', 'unknown-widget', 'action-behavior']
        for example in examples:
            with self.subTest(example=example):
                self.regeneration_example(example)

    def regeneration_example(self, example):
        retained = ROOT / 'tests/fixtures/release-de20c469'
        original = {name: (retained / name).read_bytes() for name in
                    ('api.schema.json', 'wire.rs', 'widgets.rs', 'endpoint.rs')}
        if example == 'action-behavior':
            mutant = os.environ['TRAILBOSS_COMPAT_BEHAVIOR_CANDIDATE']
            exported = subprocess.run([mutant, 'api', 'schema', '--json'],
                capture_output=True, check=True).stdout
            self.assertEqual(exported, original['api.schema.json'],
                             'This example must change behavior without changing its API schema')
            with tempfile.TemporaryDirectory() as directory:
                output = Path(directory)
                result = subprocess.run([
                    sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                    '--baseline', str(BASELINE), '--candidate', mutant,
                    '--candidate-revision', os.environ['TRAILBOSS_COMPAT_BEHAVIOR_REVISION'],
                    '--output-dir', str(output),
                ], cwd=ROOT, capture_output=True, text=True, timeout=90)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                report = json.loads((output / 'report.json').read_text())
                findings = [item for item in report.get('findings', [])
                            if item['contract'] == 'action-behavior']
                self.assertTrue(findings, report)
                self.assertTrue(any(item['evidence_scope'] == 'observed candidate Core'
                                    and item['probe'] == 'retained-independent-endpoint'
                                    and item['evidence']['consumer_exit_code'] != 0
                                    for item in findings), findings)
            return
        contract = ('widget-vocabulary' if example == 'unknown-widget' else
                    'binary-layout' if example in ('binary-field-order', 'appended-enum') else 'session-api')
        change = {'required-field': 'new required request field',
                  'method-envelope': 'new required request field',
                  'removed-method': 'removed method',
                  'binary-field-order': 'ordered binary payload changed',
                  'appended-enum': 'enum variants changed',
                  'unknown-widget': 'typed widget contract changed'}[example]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            contracts = root / 'candidate-contracts'
            contracts.mkdir()
            for name, data in original.items():
                (contracts / name).write_bytes(data)
            schema = json.loads((contracts / 'api.schema.json').read_text())
            if example == 'required-field':
                params = schema['schemas']['request']['$defs']['WorkspaceCreateParams']
                params['properties']['new_required_context'] = {'type': 'string'}
                params.setdefault('required', []).append('new_required_context')
            elif example == 'method-envelope':
                method = next(item for item in schema['schemas']['request']['oneOf']
                              if item['properties']['method']['const'] == 'workspace.get')
                method['properties']['review_header'] = {'type': 'string'}
                method.setdefault('required', []).append('review_header')
            elif example == 'removed-method':
                schema['schemas']['request']['oneOf'] = [
                    item for item in schema['schemas']['request']['oneOf']
                    if item['properties']['method']['const'] != 'session.snapshot']
            (contracts / 'api.schema.json').write_text(json.dumps(schema))
            if example == 'binary-field-order':
                path = contracts / 'wire.rs'
                path.write_text(path.read_text().replace(
                    'pub cols: u16,\n    pub rows: u16,',
                    'pub rows: u16,\n    pub cols: u16,', 1))
            elif example == 'appended-enum':
                path = contracts / 'wire.rs'
                before, after = path.read_text().rsplit(
                    '    EndpointControl { kind: String, data: String },', 1)
                path.write_text(before + '    EndpointControl { kind: String, data: String },\n'
                                '    FutureEmittedVariant { data: Vec<u8> },' + after)
            elif example == 'unknown-widget':
                path = contracts / 'widgets.rs'
                path.write_text(path.read_text().replace(
                    'pub enum Node {', 'pub enum Node {\n    FutureWidget { text: String },', 1))
            for generation in ('first', 'regenerated'):
                # Source-only adversarial evidence, never a claim that this
                # unchanged binary emitted an altered API.
                output = root / generation
                result = subprocess.run([
                    sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                    '--baseline', str(BASELINE),
                    '--candidate', os.environ['TRAILBOSS_COMPAT_CANDIDATE'],
                    '--candidate-revision', os.environ['TRAILBOSS_COMPAT_REVISION'],
                    '--candidate-contracts', str(contracts), '--output-dir', str(output),
                ], cwd=ROOT, capture_output=True, text=True, timeout=90)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertTrue((output / 'report.json').is_file(), result.stderr)
                report = json.loads((output / 'report.json').read_text())
                findings = [item for item in report['findings'] if item['contract'] == contract]
                self.assertTrue(findings, report)
                self.assertTrue(any(item['change'] == change for item in findings), findings)
                if example == 'binary-field-order':
                    self.assertTrue(any(item.get('type') == 'ClientSurfaceSize'
                                        for item in findings), findings)
                elif example == 'appended-enum':
                    self.assertTrue(any(item.get('type') == 'ServerMessage'
                                        for item in findings), findings)
                elif example == 'unknown-widget':
                    self.assertTrue(any(item.get('type') == 'Node'
                                        for item in findings), findings)
                self.assertTrue(all(item['evidence_scope'] == 'candidate source artifacts'
                                    for item in findings), findings)
                self.assertIn('unverified', report['candidate']['source_binary_correspondence'])
            for name, data in original.items():
                self.assertEqual((retained / name).read_bytes(), data)

    def test_missing_or_changed_released_artifacts_fail_closed(self):
        selected = os.environ.get('TRAILBOSS_COMPAT_ARTIFACT_CASE')
        for case in ([selected] if selected else [
                'missing-binary', 'changed-digest', 'missing-product',
                'missing-source', 'missing-schema', 'missing-consumer',
                'malformed-inventory', 'invalid-generation', 'mismatched-generation', 'mismatched-protocol']):
            with self.subTest(case=case):
                self.baseline_evidence_case(case)

    def baseline_evidence_case(self, case):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            retained = root / 'retained'
            shutil.copytree(ROOT / 'tests/fixtures/release-de20c469', retained)
            inventory_path = retained / 'inventory.json'
            inventory = json.loads(inventory_path.read_text())
            # Files in this owned deployment tree are symlinks only for cheap
            # immutable fixture reuse. Never write through a baseline link.
            baseline = root / 'deployment'
            shutil.copytree(BASELINE, baseline, copy_function=os.symlink)
            if case == 'missing-binary':
                (baseline / 'trailboss').unlink()
                expected = 'trailboss'
            elif case == 'changed-digest':
                path = baseline / 'plugins/agent-tree/agent-tree'
                path.unlink()
                path.write_bytes(b'changed artifact, not the retained producer')
                expected = 'plugins/agent-tree/agent-tree'
            elif case in ('missing-product', 'missing-source'):
                field = 'product' if case == 'missing-product' else 'source_revision'
                del inventory[field]
                expected = field
            elif case == 'malformed-inventory':
                expected = 'inventory.json'
            elif case in ('invalid-generation', 'mismatched-generation', 'mismatched-protocol'):
                field = 'protocol_version' if case == 'mismatched-protocol' else 'endpoint_generation'
                inventory[field] = ('one' if case == 'invalid-generation' else
                                    23 if case == 'mismatched-protocol' else 2)
                expected = field
            else:
                name = 'api.schema.json' if case == 'missing-schema' else 'neutral_consumer.py'
                del inventory['artifacts'][name]
                (retained / name).unlink()
                expected = name
            inventory_path.write_text('{"source_revision":' if case == 'malformed-inventory'
                                      else json.dumps(inventory))
            output = root / 'output'
            result = subprocess.run([
                sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                '--baseline', str(baseline), '--retained-contracts', str(retained),
                '--candidate', os.environ['TRAILBOSS_COMPAT_CANDIDATE'],
                '--candidate-revision', os.environ['TRAILBOSS_COMPAT_REVISION'],
                '--output-dir', str(output),
            ], cwd=ROOT, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertTrue((output / 'report.json').is_file(), result.stdout + result.stderr)
            report = json.loads((output / 'report.json').read_text())
            self.assertEqual(report['status'], 'failed', report)
            self.assertIn(expected, report['error'])
            self.assertEqual(report.get('error_contract'), 'baseline-evidence', report)
            self.assertFalse(any(item['contract'] == 'action-behavior'
                                 for item in report['findings']), report)

    def test_report_explains_likely_version_impact(self):
        selected = os.environ.get('TRAILBOSS_COMPAT_IMPACT_EXAMPLE')
        for example in ([selected] if selected else [
                'unchanged', 'addition', 'api-addition', 'removed-method', 'binary-field-order', 'unknown-widget']):
            with self.subTest(example=example):
                self.impact_example(example)

    def impact_example(self, example):
        breaking = example in ('removed-method', 'binary-field-order', 'unknown-widget')
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            extra = []
            candidate = os.environ['TRAILBOSS_COMPAT_CANDIDATE']
            revision = os.environ['TRAILBOSS_COMPAT_REVISION']
            if example == 'addition':
                candidate = os.environ['TRAILBOSS_COMPAT_CAPABLE_CANDIDATE']
                revision = os.environ['TRAILBOSS_COMPAT_CAPABLE_REVISION']
                source = Path(os.environ['TRAILBOSS_COMPAT_CAPABLE_SOURCE'])
                retained = ROOT / 'tests/fixtures/release-de20c469'
                inventory = json.loads((retained / 'inventory.json').read_text())
                contracts = output / 'candidate-contracts'
                contracts.mkdir()
                for name in ('api.schema.json', 'wire.rs', 'widgets.rs', 'endpoint.rs'):
                    shutil.copy2(source / inventory['artifacts'][name]['source_path'], contracts / name)
                extra = ['--candidate-contracts', str(contracts)]
            elif example in ('removed-method', 'api-addition'):
                contracts = output / 'candidate-contracts'
                shutil.copytree(ROOT / 'tests/fixtures/release-de20c469', contracts)
                path = contracts / 'api.schema.json'
                schema = json.loads(path.read_text())
                if example == 'api-addition':
                    variant = json.loads(json.dumps(schema['schemas']['request']['oneOf'][0]))
                    variant['properties']['method']['const'] = 'session.compatibility-inspect'
                    schema['schemas']['request']['oneOf'].append(variant)
                else:
                    schema['schemas']['request']['oneOf'] = [
                        item for item in schema['schemas']['request']['oneOf']
                        if item['properties']['method']['const'] != 'session.snapshot']
                path.write_text(json.dumps(schema))
                extra = ['--candidate-contracts', str(contracts)]
            elif example in ('binary-field-order', 'unknown-widget', 'unchanged'):
                contracts = output / 'candidate-contracts'
                shutil.copytree(ROOT / 'tests/fixtures/release-de20c469', contracts)
                if example == 'binary-field-order':
                    path = contracts / 'wire.rs'
                    path.write_text(path.read_text().replace(
                        'pub cols: u16,\n    pub rows: u16,',
                        'pub rows: u16,\n    pub cols: u16,', 1))
                elif example == 'unknown-widget':
                    path = contracts / 'widgets.rs'
                    path.write_text(path.read_text().replace(
                        'pub enum Node {', 'pub enum Node {\n    FutureWidget { text: String },', 1))
                else:
                    for name in ('wire.rs', 'widgets.rs', 'endpoint.rs'):
                        path = contracts / name
                        path.write_text('// Formatting is not a contract break.\n\n' + path.read_text()
                                        .replace('pub ', 'pub  '))
                extra = ['--candidate-contracts', str(contracts)]
            result = subprocess.run([
                sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                '--baseline', str(BASELINE),
                '--candidate', candidate,
                '--candidate-revision', revision,
                '--stable-contract-reference', '1.4.0', '--output-dir', str(output),
            ] + extra, cwd=ROOT, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 1 if breaking else 0,
                             result.stdout + result.stderr)
            report = json.loads((output / 'report.json').read_text())
            self.assertEqual(report['baseline']['product_version'], '0.1.0')
            self.assertEqual(report['version_reference']['version'], '1.4.0')
            self.assertEqual(report['version_reference']['kind'], 'hypothetical-stable-contract-reference')
            impact = report.get('version_impact', {})
            self.assertEqual(impact.get('stable_suggestion'),
                             'major' if breaking else
                             'minor' if example in ('addition', 'api-addition') else 'conditional-patch', report)
            self.assertEqual(impact['reason_code'], 'stable-public-contract-break'
                             if breaking else
                             'backward-compatible-public-addition'
                             if example in ('addition', 'api-addition') else 'no-detected-public-contract-change')
            if breaking:
                self.assertTrue(impact['provisional'])
                self.assertIn({'removed-method': 'session-api',
                               'binary-field-order': 'binary-layout',
                               'unknown-widget': 'widget-vocabulary'}[example],
                              impact['affected_contracts'])
            if example == 'addition':
                self.assertTrue(any(item['contract'] == 'named-endpoint-codec'
                    and item['impact'] == 'compatible-addition'
                    and item['name'] == 'shell.surface.equivalent.v1'
                    for item in report['findings']), report)
            elif example == 'api-addition':
                self.assertTrue(any(item['contract'] == 'session-api'
                    and item['impact'] == 'compatible-addition'
                    and item['method'] == 'session.compatibility-inspect'
                    and item['evidence_scope'] == 'candidate source artifacts'
                    for item in report['findings']), report)
            self.assertEqual(impact['scope'], 'checked-contracts')
            self.assertFalse(impact['claims_complete_compatibility'])
            self.assertIsNone(impact['selected_release_version'])

    def test_new_capabilities_preserve_released_v1_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            result = subprocess.run([
                sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                '--baseline', str(BASELINE),
                '--candidate', os.environ['TRAILBOSS_COMPAT_CAPABLE_CANDIDATE'],
                '--candidate-revision', os.environ['TRAILBOSS_COMPAT_CAPABLE_REVISION'],
                '--probe-surface-codec', 'shell.surface.equivalent.v1',
                '--output-dir', str(output),
            ], cwd=ROOT, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            report = json.loads((output / 'report.json').read_text())
            probes = {item['name']: item for item in report['probes']}
            paired = probes.get('negotiated-surface-codec', {})
            self.assertEqual(paired.get('status'), 'verified', report)
            evidence = paired['evidence']
            self.assertEqual(evidence['retained']['welcome']['surface_codec'], 'shell.surface.v1')
            self.assertEqual(evidence['capable']['welcome']['surface_codec'], 'shell.surface.equivalent.v1')
            for client in ('retained', 'capable'):
                self.assertTrue(evidence[client]['decoded_marker'])
                self.assertEqual(evidence[client]['welcome']['generation'], 1)
                self.assertEqual(evidence[client]['snapshot']['focused_tab_id'], evidence['target_tab_id'])
            self.assertEqual(probes['retained-native']['status'], 'verified')
            self.assertEqual(probes['retained-independent-endpoint']['status'], 'verified')
            self.assertEqual(report['baseline']['binary_sha256'],
                             '718444cbe83522f436c1872fbbf4efa055dae67533aeaa5a7d688618c4f83502')

    def test_public_break_remains_visible_after_package_bump(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            contracts = output / 'candidate-contracts'
            shutil.copytree(ROOT / 'tests/fixtures/release-de20c469', contracts)
            path = contracts / 'wire.rs'
            path.write_text(path.read_text()
                .replace('pub cols: u16,\n    pub rows: u16,',
                         'pub rows: u16,\n    pub cols: u16,', 1)
                .replace('PROTOCOL_VERSION: u32 = 22', 'PROTOCOL_VERSION: u32 = 23', 1))
            path = contracts / 'Cargo.toml'
            path.write_text(path.read_text().replace('version = "0.1.0"', 'version = "1.0.0"', 1))
            result = subprocess.run([
                sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                '--baseline', str(BASELINE),
                '--candidate', os.environ['TRAILBOSS_COMPAT_CANDIDATE'],
                '--candidate-revision', os.environ['TRAILBOSS_COMPAT_REVISION'],
                '--candidate-contracts', str(contracts), '--output-dir', str(output),
            ], cwd=ROOT, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            report = json.loads((output / 'report.json').read_text())
            obligation = report.get('public_path_obligation', {})
            self.assertIs(obligation.get('version_bump_restores_v1'), False, report)
            self.assertTrue(obligation['requires_preserved_v1_or_negotiated_named_codec'])
            self.assertIn('retained-generation-1-client', obligation['affected_readers'])
            self.assertEqual(report['candidate']['source_product_version'], '1.0.0')
            self.assertEqual(report['candidate']['source_protocol_version'], 23)
            self.assertEqual(report['candidate']['protocol_version'], 22)
            self.assertEqual(report['candidate']['product_version'], '0.1.0')
            self.assertEqual(report['version_impact']['stable_suggestion'], 'major')

    def test_manual_check_requires_no_release_automation_or_decision(self):
        selected = os.environ.get('TRAILBOSS_COMPAT_MANUAL_EXAMPLE')
        for outcome in ([selected] if selected else ['checked', 'incompatible', 'incomplete']):
            with self.subTest(outcome=outcome):
                self.manual_example(outcome)

    def manual_example(self, outcome):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            retained = ROOT / 'tests/fixtures/release-de20c469'
            inventory = json.loads((retained / 'inventory.json').read_text())
            for name in ('api.schema.json', 'wire.rs', 'widgets.rs', 'endpoint.rs', 'Cargo.toml'):
                destination = source / inventory['artifacts'][name]['source_path']
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(retained / name, destination)
            schema_path = source / inventory['artifacts']['api.schema.json']['source_path']
            schema = json.loads(schema_path.read_text())
            if outcome == 'incompatible':
                schema['schemas']['request']['oneOf'] = [item for item in schema['schemas']['request']['oneOf']
                    if item['properties']['method']['const'] != 'session.snapshot']
            elif outcome == 'incomplete':
                schema['schemas']['request']['$defs']['WorkspaceCreateParams']['properties']['cwd'] = {
                    'type': 'integer'}
            schema_path.write_text(json.dumps(schema))
            before = {str(path.relative_to(root)): path.read_bytes()
                      for path in root.rglob('*') if path.is_file()}
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith(('CI', 'GITHUB_', 'FORGEJO_'))}
            result = subprocess.run([
                sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                '--baseline', str(BASELINE),
                '--candidate', os.environ['TRAILBOSS_COMPAT_CANDIDATE'],
                '--candidate-revision', os.environ['TRAILBOSS_COMPAT_REVISION'],
                '--candidate-source-root', str(source), '--output-dir', str(root / 'output'),
            ], cwd=root, env=env, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 0 if outcome == 'checked' else 1,
                             result.stdout + result.stderr)
            report = json.loads((root / 'output/report.json').read_text())
            if outcome == 'incompatible':
                self.assertTrue(any(item['contract'] == 'session-api' and item.get('method') == 'session.snapshot'
                                    for item in report['findings']), report)
                self.assertEqual(report['version_impact']['stable_suggestion'], 'major')
            elif outcome == 'incomplete':
                self.assertTrue(any(item['impact'] == 'unverified-change' for item in report['findings']), report)
                self.assertIsNone(report['version_impact']['stable_suggestion'])
            else:
                self.assertFalse(report['findings'], report)
                self.assertEqual(report['version_impact']['stable_suggestion'], 'conditional-patch')
            self.assertIsNone(report['version_impact']['selected_release_version'])
            self.assertIsNone(report['version_impact']['pre_one']['selected_policy'])
            after = {str(path.relative_to(root)): path.read_bytes() for path in root.rglob('*')
                     if path.is_file() and 'output' not in path.relative_to(root).parts}
            self.assertEqual(after, before, 'Manual check changed inputs or created release/decision/VCS configuration')

    def test_report_does_not_claim_unexercised_contracts(self):
        selected = os.environ.get('TRAILBOSS_COMPAT_COVERAGE_EXAMPLE')
        for example in ([selected] if selected else [
                'no-source', 'param-type', 'mixed', 'response-field', 'union-required']):
            with self.subTest(example=example):
                self.coverage_example(example)

    def coverage_example(self, example):
        changed = example != 'no-source'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            extra = []
            if changed:
                contracts = output / 'candidate-contracts'
                shutil.copytree(ROOT / 'tests/fixtures/release-de20c469', contracts)
                path = contracts / 'api.schema.json'
                schema = json.loads(path.read_text())
                if example in ('param-type', 'mixed'):
                    schema['schemas']['request']['$defs']['WorkspaceCreateParams']['properties']['cwd'] = {
                        'type': 'integer'}
                if example == 'union-required':
                    context = schema['schemas']['request']['$defs']['ConversationContext']
                    context.setdefault('required', []).append('reset_entry_id')
                if example == 'mixed':
                    variant = json.loads(json.dumps(schema['schemas']['request']['oneOf'][0]))
                    variant['properties']['method']['const'] = 'session.compatibility-inspect'
                    schema['schemas']['request']['oneOf'].append(variant)
                if example == 'response-field':
                    found = False
                    for name, section in schema['schemas'].items():
                        if name != 'request' and 'WorkspaceInfo' in section.get('$defs', {}):
                            section['$defs']['WorkspaceInfo']['properties']['future_optional_output'] = {
                                'type': 'string'}
                            found = True
                    self.assertTrue(found, 'Pinned public output schema must contain WorkspaceInfo')
                path.write_text(json.dumps(schema))
                extra = ['--candidate-contracts', str(contracts)]
            result = subprocess.run([
                sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                '--baseline', str(BASELINE),
                '--candidate', os.environ['TRAILBOSS_COMPAT_CAPABLE_CANDIDATE'],
                '--candidate-revision', os.environ['TRAILBOSS_COMPAT_CAPABLE_REVISION'],
                '--output-dir', str(output),
            ] + extra, cwd=ROOT, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 1 if changed else 0,
                             result.stdout + result.stderr)
            report = json.loads((output / 'report.json').read_text())
            coverage = report.get('coverage', {})
            self.assertEqual(coverage.get('overall'), 'bounded-evidence', report)
            self.assertEqual(coverage['direction'], 'retained-client -> candidate-Core')
            self.assertEqual(coverage['accepted_endpoint_generation'], 1)
            self.assertIn('plugin.action.invoke', coverage['methods'])
            self.assertIn('shell.surface.v1', coverage['codecs'])
            self.assertIn('label', coverage['observed_widget_nodes'])
            for name in ('graphics', 'popup', 'surface-delta', 'surface-reuse', 'surface-scroll'):
                self.assertEqual(coverage['unverified'][name], 'not-exercised')
            self.assertEqual(coverage['structural_contracts'], 'source-artifacts-compared'
                             if changed else 'unverified')
            self.assertIsNone(report['version_impact']['stable_suggestion'])
            self.assertEqual(report['version_impact']['reason_code'], 'unclassified-public-schema-change'
                             if changed else 'insufficient-public-contract-evidence')
            if changed:
                self.assertEqual(coverage['unverified']['session-api'], 'unclassified-schema-change')
                self.assertTrue(any(item['contract'] == 'session-api'
                    and item['impact'] == 'unverified-change'
                    and item['evidence_scope'] == 'candidate source artifacts'
                    for item in report['findings']), report)
                if example == 'mixed':
                    self.assertTrue(any(item['impact'] == 'compatible-addition'
                                        for item in report['findings']), report)

    def test_pre_one_policy_is_not_invented(self):
        selected = os.environ.get('TRAILBOSS_COMPAT_PRE_ONE_EXAMPLE')
        for example in ([selected] if selected else ['unchanged', 'addition', 'public-break']):
            with self.subTest(example=example):
                self.pre_one_example(example)

    def pre_one_example(self, example):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            contracts = output / 'candidate-contracts'
            shutil.copytree(ROOT / 'tests/fixtures/release-de20c469', contracts)
            candidate, revision = os.environ['TRAILBOSS_COMPAT_CANDIDATE'], os.environ['TRAILBOSS_COMPAT_REVISION']
            if example == 'public-break':
                path = contracts / 'wire.rs'
                path.write_text(path.read_text().replace(
                    'pub cols: u16,\n    pub rows: u16,',
                    'pub rows: u16,\n    pub cols: u16,', 1))
            elif example == 'addition':
                path = contracts / 'api.schema.json'
                schema = json.loads(path.read_text())
                variant = json.loads(json.dumps(schema['schemas']['request']['oneOf'][0]))
                variant['properties']['method']['const'] = 'session.compatibility-inspect'
                schema['schemas']['request']['oneOf'].append(variant)
                path.write_text(json.dumps(schema))
            version_before = (contracts / 'Cargo.toml').read_bytes()
            result = subprocess.run([
                sys.executable, str(ROOT / 'tests/support/release_compatibility.py'),
                '--baseline', str(BASELINE),
                '--candidate', candidate,
                '--candidate-revision', revision,
                '--candidate-contracts', str(contracts), '--output-dir', str(output),
            ], cwd=ROOT, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 1 if example == 'public-break' else 0,
                             result.stdout + result.stderr)
            report = json.loads((output / 'report.json').read_text())
            self.assertEqual(report['baseline']['product_version'], '0.1.0')
            self.assertEqual(report['candidate']['product_version'], '0.1.0')
            if example == 'addition':
                self.assertTrue(any(item['impact'] == 'compatible-addition'
                    and item['evidence_scope'] == 'candidate source artifacts'
                    for item in report['findings']), report)
            implication = {'unchanged': 'conditional-patch', 'addition': 'minor', 'public-break': 'major'}[example]
            self.assertEqual(report['version_impact']['stable_suggestion'], implication)
            policy = report['version_impact'].get('pre_one', {})
            self.assertEqual(policy.get('status'), 'owner-policy-required', report)
            self.assertIsNone(policy['selected_policy'])
            self.assertIsNone(policy['suggested_product_bump'])
            self.assertEqual(policy['stable_contract_implication'], implication)
            self.assertIsNone(report['version_impact']['selected_release_version'])
            self.assertEqual((contracts / 'Cargo.toml').read_bytes(), version_before,
                             'The command applied a version policy instead of leaving that choice outside')


if __name__ == '__main__':
    unittest.main()
