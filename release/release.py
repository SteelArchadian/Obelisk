"""Build a signed Obelisk release and lay it out for GitHub.

  python -I release/release.py "what changed"            # next patch version (or the current one if not released yet)
  python -I release/release.py "what changed" --version 0.3.0

First it checks the source for private names, runs the core tests and svelte-check; after the build it scans the
installer with Windows Defender and commits (unless --no-commit). Any failure stops it with a short reason.

Run from a shell where cargo, npm and the MinGW toolchain are on PATH. The updater signing key is read from
OBELISK_SIGNING_KEY (a path), default %USERPROFILE%\\.tauri\\obelisk-updater.key; it never goes in the repo.

Leaves release/<version>/ holding:
  Obelisk_<version>_x64-setup.exe   the installer
  Obelisk_<version>_x64-setup.exe.sig
  latest.json                       for the updater (points at the v<version> GitHub release)
  notes.md                          the release notes
Publish them as the GitHub release tagged v<version>, with latest.json among its assets.
"""
import datetime
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
APP = os.path.join(ROOT, 'app')
TAURI = os.path.join(APP, 'src-tauri')
CONF = os.path.join(TAURI, 'tauri.conf.json')
CARGO = os.path.join(TAURI, 'Cargo.toml')
REPO = 'SteelArchadian/Obelisk'
DEFENDER = r'C:\Program Files\Windows Defender\MpCmdRun.exe'


def current_version():
    with open(CONF, encoding='utf-8') as f:
        return json.load(f)['version']


def set_version(v):
    with open(CONF, encoding='utf-8') as f:
        conf = json.load(f)
    conf['version'] = v
    with open(CONF, 'w', encoding='utf-8') as f:
        json.dump(conf, f, indent=2)
        f.write('\n')
    with open(CARGO, encoding='utf-8') as f:
        text = f.read()
    text = re.sub(r'(?m)^version = "[^"]*"', f'version = "{v}"', text, count=1)
    with open(CARGO, 'w', encoding='utf-8') as f:
        f.write(text)


def next_version():
    v = current_version()
    if not os.path.exists(os.path.join(HERE, v)):
        return v
    major, minor, patch = (int(x) for x in v.split('.'))
    return f'{major}.{minor}.{patch + 1}'


def private_words():
    """Names that must never go public (the git identity on this PC, plus OBELISK_PRIVATE_WORDS, comma-separated)."""
    words = [w.strip() for w in os.environ.get('OBELISK_PRIVATE_WORDS', '').split(',') if w.strip()]
    name = subprocess.run(['git', 'config', '--global', 'user.name'], capture_output=True, text=True).stdout.strip()
    return [w for w in words + [name] if w]


def checks():
    """Everything that must pass before a build: no private names in the source, core tests, Svelte check."""
    words = private_words()
    for root in (os.path.join(APP, 'src'), os.path.join(TAURI, 'src'), os.path.join(ROOT, 'core', 'src')):
        for dirpath, _, files in os.walk(root):
            for name in files:
                path = os.path.join(dirpath, name)
                with open(path, encoding='utf-8', errors='replace') as f:
                    text = f.read().lower()
                hit = next((w for w in words if w.lower() in text), None)
                if hit:
                    sys.exit(f'a private name is in {os.path.relpath(path, ROOT)}; take it out first')
    tests = subprocess.run(['cargo', 'test', '--lib', '--quiet'], cwd=os.path.join(ROOT, 'core'), capture_output=True, text=True)
    if tests.returncode != 0:
        sys.exit(f'core tests failed:\n{(tests.stdout + tests.stderr)[-1500:]}')
    npx = shutil.which('npx') or 'npx'
    svelte = subprocess.run([npx, 'svelte-check', '--threshold', 'error', '--output', 'machine'], cwd=APP, capture_output=True, text=True)
    if svelte.returncode != 0:
        sys.exit(f'svelte-check failed:\n{svelte.stdout[-1500:]}')


def commit(version, notes):
    subprocess.run(['git', 'add', '-A'], cwd=ROOT, check=True)
    subprocess.run(['git', 'commit', '-q', '-m', f'{version}: {notes}'], cwd=ROOT, check=True)
    return subprocess.run(['git', 'log', '-1', '--format=%h'], cwd=ROOT, capture_output=True, text=True).stdout.strip()


def main():
    sys.stdout.reconfigure(encoding='utf-8', errors='replace')
    args = sys.argv[1:]
    version = None
    if '--version' in args:
        i = args.index('--version')
        version = args[i + 1]
        del args[i:i + 2]
    flags = [a for a in args if a.startswith('--')]
    args = [a for a in args if not a.startswith('--')] + flags
    if not args or args[0].startswith('--') or not args[0].strip():
        sys.exit('give the release notes: release.py "what changed"')
    notes = args[0].strip()
    version = version or next_version()
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        sys.exit(f'not a version: {version}')
    out = os.path.join(HERE, version)
    if os.path.exists(out):
        sys.exit(f'release/{version} already exists')

    checks()
    key = os.environ.get('OBELISK_SIGNING_KEY') or os.path.join(os.path.expanduser('~'), '.tauri', 'obelisk-updater.key')
    if not os.path.isfile(key):
        sys.exit(f'no signing key at {key}')
    set_version(version)
    env = dict(os.environ, TAURI_SIGNING_PRIVATE_KEY=key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD='')
    log = os.path.join(os.environ.get('TEMP', HERE), f'obelisk-build-{version}.log')
    print(f'building {version} (log: {log})')
    with open(log, 'w', encoding='utf-8') as f:
        npx = shutil.which('npx') or 'npx'
        code = subprocess.run([npx, 'tauri', 'build'], cwd=APP, env=env, stdout=f, stderr=subprocess.STDOUT).returncode
    if code != 0:
        sys.exit(f'build failed (exit {code}); see {log}')

    nsis = os.path.join(TAURI, 'target', 'release', 'bundle', 'nsis')
    setup = f'Obelisk_{version}_x64-setup.exe'
    for name in (setup, setup + '.sig'):
        if not os.path.isfile(os.path.join(nsis, name)):
            sys.exit(f'the build left no {name}')
    scan = subprocess.run([DEFENDER, '-Scan', '-ScanType', '3', '-File', os.path.join(nsis, setup), '-DisableRemediation'],
                          capture_output=True, text=True)
    if scan.returncode != 0:
        sys.exit(f'Defender flagged or could not scan the installer:\n{scan.stdout[-600:]}')

    os.makedirs(out)
    for name in (setup, setup + '.sig'):
        shutil.copy2(os.path.join(nsis, name), os.path.join(out, name))
    with open(os.path.join(out, setup + '.sig'), encoding='utf-8') as f:
        signature = f.read().strip()
    latest = {
        'version': version,
        'notes': notes,
        'pub_date': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'platforms': {
            'windows-x86_64': {
                'signature': signature,
                'url': f'https://github.com/{REPO}/releases/download/v{version}/{setup}',
            },
        },
    }
    with open(os.path.join(out, 'latest.json'), 'w', encoding='utf-8') as f:
        json.dump(latest, f, indent=2)
        f.write('\n')
    with open(os.path.join(out, 'notes.md'), 'w', encoding='utf-8') as f:
        f.write(f'# Obelisk {version}\n\n{notes}\n')
    head = commit(version, notes) if '--no-commit' not in args else '(not committed)'
    print(f'release/{version} ready and Defender-clean: commit {head}, tag v{version}, assets {setup}, {setup}.sig, latest.json')


if __name__ == '__main__':
    main()
