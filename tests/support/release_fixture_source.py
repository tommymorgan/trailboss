"""Prepare an owned, unpublished real-Core source fixture; never build or install.

Run with an absent output directory. Build its source/ directory using the normal
project toolchain, then pass the resulting executable to the compatibility tests.
The fixture is a real pinned Trailboss Core, not a substitute server.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import uuid


BASE = 'de20c4694668d6305e6ff650c47cfa3614abd5d0'
ROOT = Path(__file__).resolve().parents[2]


def replace(path, before, after):
    original = path.read_text()
    if original.count(before) != 1:
        raise ValueError('Pinned fixture source no longer matches exactly: ' + str(path))
    path.write_text(original.replace(before, after, 1))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kind', choices=['action-break', 'codec-alias'])
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = output / 'source'
    workspace = 'release-' + args.kind + '-' + uuid.uuid4().hex[:12]
    subprocess.run(['jj', 'workspace', 'add', '--name', workspace,
                    '--revision', BASE, str(source)], cwd=ROOT, check=True)
    subprocess.run(['jj', 'new', BASE], cwd=source, check=True)
    runtime = source / 'crates/trailboss-runtime/src'
    if args.kind == 'action-break':
        path = runtime / 'app/api/tabs.rs'
        replace(path, '        self.state.switch_workspace_tab(ws_idx, tab_idx);\n'
                '        let tab = self.tab_info(ws_idx, tab_idx).unwrap();',
                '        let tab = self.tab_info(ws_idx, tab_idx).unwrap();')
        changed = [path]
    else:
        endpoint = runtime / 'protocol/endpoint.rs'
        transport = runtime / 'server/client_transport.rs'
        replace(endpoint, 'pub const SURFACE_CODEC_V1: &str = "shell.surface.v1";',
                'pub const SURFACE_CODEC_V1: &str = "shell.surface.v1";\n'
                'pub const SURFACE_CODEC_EQUIVALENT_V1: &str = "shell.surface.equivalent.v1";')
        replace(transport, '    let (\n        client_cols,',
                '    let mut selected_surface_codec = crate::protocol::endpoint::SURFACE_CODEC_V1;\n'
                '    let (\n        client_cols,')
        replace(transport, '            (\n                hello.surface_size.cols,',
                '            if hello\n'
                '                .surface_codecs\n'
                '                .iter()\n'
                '                .any(|codec| codec == crate::protocol::endpoint::SURFACE_CODEC_EQUIVALENT_V1)\n'
                '            {\n'
                '                selected_surface_codec = crate::protocol::endpoint::SURFACE_CODEC_EQUIVALENT_V1;\n'
                '            }\n            (\n                hello.surface_size.cols,')
        replace(transport, '        let welcome = EndpointServerWelcome::compatible(',
                '        let mut welcome = EndpointServerWelcome::compatible(')
        replace(transport, '        ServerMessage::EndpointControl {\n            kind: ENDPOINT_WELCOME_KIND.into(),',
                '        if welcome.surface_codec != selected_surface_codec {\n'
                '            welcome.surface_codec.clear();\n'
                '            welcome.surface_codec.push_str(selected_surface_codec);\n'
                '        }\n'
                '        ServerMessage::EndpointControl {\n            kind: ENDPOINT_WELCOME_KIND.into(),')
        changed = [endpoint, transport]
    revision = subprocess.check_output(['jj', 'log', '-r', '@', '--no-graph', '-T', 'commit_id'],
                                       cwd=source, text=True).strip()
    receipt = {'base_revision': BASE, 'fixture_revision': revision, 'kind': args.kind,
               'workspace_name': workspace,
               'source': str(source), 'changed_source_sha256': {
                   str(path.relative_to(source)): hashlib.sha256(path.read_bytes()).hexdigest()
                   for path in changed}, 'built': False}
    (output / 'source-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
