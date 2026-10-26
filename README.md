# Obelisk

A friendly Minecraft modpack manager for Windows, built on top of [Prism Launcher](https://prismlauncher.org/).
Prism stays the engine (Microsoft sign-in, Java, launching). Obelisk is the everyday front end: your
modpacks, their mods, updates, crash help and sharing, in one calm window.

## What it does

- **Library.** Every Prism instance as a tile or list row, with play time, last played and a one-click Play.
  Sync brings in CurseForge app profiles and other launchers' instances.
- **Content.** Mods, resource packs, shader packs, data packs and worlds per profile. Enable, disable, delete
  (deleted files go to a profile trash you can restore from), bulk select and sort.
- **Browse and install** from Modrinth and CurseForge in one search, with project pages, galleries,
  dependencies, and Fabric mods on NeoForge through Sinytra Connector when you allow it.
- **Updates** for Modrinth and CurseForge files, including files CurseForge won't let other apps download:
  Obelisk uses Modrinth's byte-identical copy when there is one, or opens the CurseForge page so you can
  click Download and then moves the file into place after checking its hash.
- **Duplicate guard.** A mod installed twice is flagged in red and the game won't start until you pick
  which copy to keep (or choose to ignore it).
- **Diagnosis.** Reads crash reports and logs, says in plain words what went wrong and which mod is to
  blame, and shows what slowed loading down.
- **Backups.** Automatic checkpoints before changes, plus manual ones, restorable in a click.
- **Sharing.**
  - *Share code:* a 6-character code a friend pastes into Import. It works for 30 minutes. Files that
    Modrinth or CurseForge host come from there; the rest come straight from your PC. Files that no site
    vouches for are listed first, and your friend chooses whether to take them.
  - *Export as zip:* the whole pack in one file in your Downloads.
  - *Collaborative packs:* friends keep a pack in step, and every change can be reviewed and undone.
  - *Server and client packs* from a profile.
- **Themes**, a tray icon, start with Windows, and a choice of where modpacks live on disk.
- **Updates itself** from this repository's releases. Each release is signed and the signature is checked
  before anything installs.

### For AI assistants (optional, off by default)

Settings → Plugins → AI assistant connects Claude or ChatGPT (through Codex) to Obelisk over a local MCP
server (`obelisk.exe --mcp`). The assistant can do what the app's own window can do, with a few things
kept for you alone: API keys, signing out, and permanent deletes. It listens on 127.0.0.1 only and needs
a per-session token.

The same connection can run Minecraft *test copies* for mod development: launch in the background without
taking the mouse, aim the camera, fast-quit, profile, and gate candidate builds. That part needs a test
bench folder described in `%USERPROFILE%\.obelisk\bench.json`; without it, those commands simply say no
bench is set up.

```json
{
  "root": "C:\\path\\to\\test-copies",
  "install": "C:/path/to/minecraft-runtime",
  "harness": "C:/path/to/testharness.jar",
  "scripts": "C:/path/to/bench-scripts",
  "play": "MyPack",
  "guarded": []
}
```

## Install

Download `Obelisk_<version>_x64-setup.exe` from the [latest release](https://github.com/SteelArchadian/Obelisk/releases/latest)
and run it. You'll need [Prism Launcher](https://prismlauncher.org/download/windows/) installed for your
user account (its default install), since Obelisk works with Prism's instances and launches through it.

Windows may warn about an unrecognised publisher, because the installer isn't code-signed with a paid
certificate. Updates are signed separately, with Obelisk's own update key.

## Build from source

Obelisk is a Tauri 2 app: a Rust core (`core/`) and a SvelteKit + Svelte 5 interface (`app/`).

Toolchain: Rust's GNU toolchain (pinned by `rust-toolchain.toml`), MinGW binutils on PATH (for example
WinLibs through winget), Node.js 20+. No Visual Studio needed.

```bash
cd app
npm install
npx tauri build
```

The installer lands in `app/src-tauri/target/release/bundle/nsis/`.

Checks:

```bash
cd core && cargo test --lib                              # data layer, offline
cd app && npx svelte-check --tsconfig ./tsconfig.json    # interface types
```

### Releases

`python -I release/release.py "what changed"` builds a signed release into `release/<version>/`
(installer, `.sig`, `latest.json`, notes). The updater signing key is read from outside the repository
(`OBELISK_SIGNING_KEY`, default `%USERPROFILE%\.tauri\obelisk-updater.key`). Those files are published
as the GitHub release `v<version>`.

## Also in this repository

`obelisk.py` is Obelisk's first form: a dependency-free Python script that fetches the CurseForge files
whose authors turned off third-party downloads, from Modrinth when a byte-identical copy exists, otherwise
through your browser. `python -m unittest -v` runs its offline tests.

## License

Obelisk is free software under the GNU Lesser General Public License, version 3 only (LGPL-3.0-only).
See `COPYING.LESSER` and `COPYING`.

Made by SteelArchadian, with AI assistance.

Obelisk isn't affiliated with Mojang, Microsoft, Prism Launcher, Modrinth or CurseForge.
