# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Black-box installer tests, with offline release downloads and real native binary option."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path)
args = parser.parse_args()
installer = Path(__file__).resolve().parents[1] / 'install.sh'
system = {'Linux': 'unknown-linux-gnu', 'Darwin': 'apple-darwin'}[platform.system()]
machine = {'x86_64': 'x86_64', 'aarch64': 'aarch64', 'arm64': 'aarch64'}[platform.machine()]
archive_name = f'codesight-mcp-v0.1.1-{machine}-{system}.tar.gz'
with tempfile.TemporaryDirectory(prefix='codesight-installer-') as tmp:
    root = Path(tmp)
    downloads = root / 'downloads'
    downloads.mkdir()
    payload = root / 'codesight-mcp'
    if args.binary:
        shutil.copyfile(args.binary.resolve(), payload)
    else:
        payload.write_text('#!/bin/sh\nprintf "codesight-mcp 0.1.1\\n"\n')
    payload.chmod(0o755)
    archive = downloads / archive_name
    with tarfile.open(archive, 'w:gz') as bundle:
        bundle.add(payload, arcname='codesight-mcp')
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = downloads / f'{archive_name}.sha256'
    checksum.write_text(f'{digest}  {archive_name}\n')
    mock_bin = root / 'mock-bin'
    mock_bin.mkdir()
    # Only the network boundary is replaced. All extraction, hash checking,
    # version validation, staging and replacement use the actual installer.
    (mock_bin / 'curl').write_text('''#!/bin/sh
set -eu
for arg do
  case "$arg" in
    https://github.com/olegpustovit/codesight-mcp/releases/download/v0.1.1/*) url=$arg ;;
  esac
  if [ "${previous:-}" = -o ]; then destination=$arg; fi
  previous=$arg
done
[ -n "${url:-}" ]
cp "$TEST_DOWNLOADS/${url##*/}" "$destination"
''')
    (mock_bin / 'curl').chmod(0o755)
    destination = root / 'install path with spaces'
    env = {**os.environ, 'PATH': f'{mock_bin}{os.pathsep}{os.environ["PATH"]}',
           'TEST_DOWNLOADS': str(downloads), 'CODESIGHT_INSTALL_DIR': str(destination)}
    def install(version='v0.1.1'):
        return subprocess.run(['sh', str(installer), version], env=env, text=True, capture_output=True, timeout=20)

    result = install()
    assert result.returncode == 0, result.stderr
    installed = destination / 'codesight-mcp'
    assert installed.read_bytes() == payload.read_bytes()
    assert subprocess.check_output([str(installed), '--version'], text=True).strip() == 'codesight-mcp 0.1.1'
    # An update corrupted in transit must fail without removing the working binary.
    before = installed.read_bytes()
    archive.write_bytes(archive.read_bytes() + b'corrupt-download')
    result = install()
    assert result.returncode != 0, 'corrupt download accepted'
    assert installed.read_bytes() == before, 'failed update replaced existing binary'
    result = install('../untrusted')
    assert result.returncode != 0, 'invalid version accepted'
    assert installed.read_bytes() == before
    env['CODESIGHT_INSTALL_DIR'] = 'relative/path'
    assert install().returncode != 0, 'relative installation path accepted'
    print('PASS: native install, path with spaces, checksum tampering, failed-update preservation, invalid inputs')
