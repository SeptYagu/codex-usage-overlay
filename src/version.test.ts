import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { APP_VERSION } from './version';

const root = resolve(process.cwd());

function readText(relative: string): string {
  return readFileSync(resolve(root, relative), 'utf8');
}

function readJson(relative: string): Record<string, unknown> {
  return JSON.parse(readText(relative)) as Record<string, unknown>;
}

/** The `version` of the `[package]` section of a Cargo manifest. */
function cargoPackageVersion(relative: string): string | undefined {
  const manifest = readText(relative);
  const section = manifest
    .split(/^\[package\]\s*$/m)[1]
    ?.split(/^\[/m)[0];
  return section?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
}

/** The `version` of this crate inside a Cargo lock file. */
function cargoLockVersion(relative: string): string | undefined {
  const lock = readText(relative);
  const block = lock
    .split(/\[\[package\]\]\s*\nname = "codex-usage-overlay"\s*\n/)[1]
    ?.split(/\[\[package\]\]/)[0];
  return block?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
}

describe('the app version', () => {
  // The version is spelled out in six places. A release that updates only
  // some of them ships an installer whose settings window and tray menu show
  // the previous version, so the agreement is a release gate, not a
  // convention.
  it('is identical in every release location', () => {
    const packageJson = readJson('package.json');
    const packageLock = readJson('package-lock.json');
    const rootPackage = (packageLock.packages as Record<string, { version?: string }>)[''];

    const locations: Record<string, string | undefined> = {
      'package.json': packageJson.version as string,
      'package-lock.json': packageLock.version as string,
      'package-lock.json (root package)': rootPackage?.version,
      'src-tauri/Cargo.toml': cargoPackageVersion('src-tauri/Cargo.toml'),
      'src-tauri/Cargo.lock': cargoLockVersion('src-tauri/Cargo.lock'),
      'src-tauri/tauri.conf.json': (readJson('src-tauri/tauri.conf.json').version as string),
      'src/version.ts': APP_VERSION,
    };

    for (const [name, value] of Object.entries(locations)) {
      expect(value, `${name} must carry the release version`).toBe(APP_VERSION);
    }
  });

  it('uses a semantic version without a pre-release suffix', () => {
    expect(APP_VERSION).toMatch(/^\d+\.\d+\.\d+$/);
  });
});
