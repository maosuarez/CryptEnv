#!/usr/bin/env node
// Stages the Windows installer's extra payload before `tauri build`:
//
//   src-tauri/binaries/crypt-env-<triple>.exe       (externalBin sidecar)
//   src-tauri/binaries/crypt-env-mcp-<triple>.exe   (externalBin sidecar)
//   src-tauri/resources/wsl/crypt-env-setup         (static musl WSL helper)
//
// Consumed by `pnpm tauri:build:windows`, which merges
// src-tauri/tauri.windows-bundle.conf.json. The base tauri.conf.json never
// references these files, so `pnpm tauri dev` and the macOS/Linux bundles are
// unaffected (openspec change windows-installer-cli-and-wsl-panel, D1).
//
// The musl helper cannot be built with the Windows toolchain. Provide it via
// CRYPTENV_WSL_HELPER=<path> (CI downloads it from the Linux job), or build it
// from WSL and copy it into src-tauri/resources/wsl/:
//   cargo build --release -p crypt-env-setup --target x86_64-unknown-linux-musl

import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, openSync, readFileSync, readSync, closeSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const tauriDir = join(root, 'src-tauri');

function fail(msg) {
  console.error(`prepare-windows-bundle: ${msg}`);
  process.exit(1);
}

if (process.platform !== 'win32') {
  fail('the Windows bundle can only be prepared on Windows (MSVC toolchain).');
}

const run = (cmd, args) =>
  execFileSync(cmd, args, { cwd: tauriDir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });

// 1. Build both CLI binaries from this exact source revision.
execFileSync('cargo', ['build', '--release', '--bin', 'crypt-env', '--bin', 'crypt-env-mcp'], {
  cwd: tauriDir,
  stdio: 'inherit',
});

const triple = /^host:\s*(\S+)$/m.exec(run('rustc', ['-vV']))?.[1];
if (!triple) fail('cannot determine the rustc host target triple');
const targetDir = JSON.parse(run('cargo', ['metadata', '--no-deps', '--format-version', '1'])).target_directory;

// 2. Stage them as Tauri sidecars (`<name>-<triple>.exe`).
const binDir = join(tauriDir, 'binaries');
mkdirSync(binDir, { recursive: true });
for (const name of ['crypt-env', 'crypt-env-mcp']) {
  const src = join(targetDir, 'release', `${name}.exe`);
  if (!existsSync(src)) fail(`missing build output ${src}`);
  copyFileSync(src, join(binDir, `${name}-${triple}.exe`));
}

// 3. Packaging check: the bundled CLI reports the same version as the GUI.
const guiVersion = JSON.parse(readFileSync(join(tauriDir, 'tauri.conf.json'), 'utf8')).version;
const cliVersion = execFileSync(join(binDir, `crypt-env-${triple}.exe`), ['--version'], { encoding: 'utf8' }).trim();
if (cliVersion !== `crypt-env ${guiVersion}`) {
  fail(`version mismatch: bundled CLI reports "${cliVersion}", GUI is ${guiVersion}`);
}

// 4. Stage the static WSL helper.
const helperDst = join(tauriDir, 'resources', 'wsl', 'crypt-env-setup');
mkdirSync(dirname(helperDst), { recursive: true });
if (process.env.CRYPTENV_WSL_HELPER) {
  copyFileSync(resolve(process.env.CRYPTENV_WSL_HELPER), helperDst);
}
if (!existsSync(helperDst) || statSync(helperDst).size === 0) {
  fail(
    `missing WSL helper at ${helperDst}. Build it in WSL with\n` +
      '  cargo build --release -p crypt-env-setup --target x86_64-unknown-linux-musl\n' +
      'and copy it there, or set CRYPTENV_WSL_HELPER to its path.',
  );
}
const magic = Buffer.alloc(4);
const fd = openSync(helperDst, 'r');
readSync(fd, magic, 0, 4, 0);
closeSync(fd);
if (!magic.equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
  fail(`${helperDst} is not a Linux ELF executable`);
}

console.log(`prepare-windows-bundle: staged ${cliVersion} sidecars (${triple}) and the WSL helper`);
