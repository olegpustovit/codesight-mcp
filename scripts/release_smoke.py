# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Native release check: version, fresh ingest, MCP handshake, tools and content."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import subprocess
import tempfile


async def smoke(binary: Path, version: str, check_cli: bool) -> None:
    if check_cli:
        result = subprocess.run([str(binary), '--version'], text=True, capture_output=True, check=True, timeout=10)
        assert result.stdout.strip() == f'dowser-mcp {version}', result.stdout
    with tempfile.TemporaryDirectory(prefix='dowser-release-') as tmp:
        root = Path(tmp)
        workspace = root / 'workspace'
        workspace.mkdir()
        source = 'def release_greeting(name):\n    return "Hello, " + name\n'
        (workspace / 'greeting.py').write_text(source)
        env = {**os.environ, 'DOWSER_INDEX_DIR': str(root / 'index'), 'RUST_LOG': 'warn'}
        subprocess.run([str(binary), 'ingest', str(workspace)], env=env, capture_output=True, check=True, timeout=30)
        with (root / 'stderr.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(
                str(binary), 'serve', '--workspace', str(workspace),
                cwd=root, env=env, stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE, stderr=log,
            )
            async def send(message):
                process.stdin.write((json.dumps({'jsonrpc': '2.0', **message}) + '\n').encode())
                await process.stdin.drain()

            async def receive(request_id):
                while True:
                    line = await asyncio.wait_for(process.stdout.readline(), timeout=10)
                    assert line, f'server closed stdout (exit={process.returncode})'
                    response = json.loads(line)
                    if response.get('id') == request_id:
                        assert 'error' not in response, response
                        return response['result']

            def envelope(result):
                assert not result.get('isError', False), result
                blocks = [part['text'] for part in result.get('content', []) if part.get('type') == 'text']
                assert blocks, result
                data = json.loads(blocks[0])
                assert data.get('ok') is True, data
                return data

            try:
                await send({'id': 1, 'method': 'initialize', 'params': {
                    'protocolVersion': '2024-11-05', 'capabilities': {},
                    'clientInfo': {'name': 'native-release-smoke', 'version': '1'},
                }})
                initialized = await receive(1)
                assert initialized['serverInfo']['version'] == version, initialized
                await send({'method': 'notifications/initialized'})
                # Send immediately after notification: detects blocking stdio regressions.
                await send({'id': 2, 'method': 'tools/list', 'params': {}})
                tools = await receive(2)
                names = {tool['name'] for tool in tools['tools']}
                assert names == {'search', 'outline', 'read', 'references', 'impact', 'health'}, names
                await send({'id': 3, 'method': 'tools/call', 'params': {
                    'name': 'outline', 'arguments': {'view': 'file', 'path': 'greeting.py'},
                }})
                outline = envelope(await receive(3))
                assert 'release_greeting' in json.dumps(outline['data']), outline
                await send({'id': 4, 'method': 'tools/call', 'params': {
                    'name': 'read', 'arguments': {'refs': ['greeting.py:1-2']},
                }})
                read = envelope(await receive(4))
                assert 'Hello, ' in json.dumps(read['data']), read
                assert 'release_greeting' in json.dumps(read['data']), read
                print(json.dumps({'ok': True, 'binary': str(binary), 'version': version, 'tools': sorted(names),
                                  'checks': ['version', 'fresh-ingest', 'initialize', 'tools/list', 'outline', 'read']}))
            except BaseException:
                print((root / 'stderr.log').read_text(errors='replace')[-8192:])
                raise
            finally:
                if process.returncode is None:
                    process.terminate()
                    try:
                        await asyncio.wait_for(process.wait(), timeout=5)
                    except TimeoutError:
                        process.kill()
                        await process.wait()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--version', default='0.2.0')
    parser.add_argument('--skip-cli-version', action='store_true', help='Diagnostic only for pre-release binaries lacking --version')
    args = parser.parse_args()
    asyncio.run(smoke(args.binary.resolve(), args.version, not args.skip_cli_version))
