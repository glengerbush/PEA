#!/usr/bin/env node
// Single source of truth for the app version is the root package.json. Two
// consumers read it directly and need no touching here:
//   - the frontend Footer  (imports ../../../../package.json)
//   - tauri.conf.json       ("version": "../../../package.json" → the updater)
// Everything else is a TOML/bash/JSON copy that can't auto-derive, so this
// script rewrites those in lockstep. Run it, then `git tag vX.Y.Z && git push`.
//
//   node scripts/set-version.mjs 1.2.0   # set version, propagate everywhere
//   node scripts/set-version.mjs         # re-sync everything to root package.json
//
// It only ever touches the version line in each file, so diffs stay minimal.

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

// Target version: the CLI arg, or the current root package.json version (re-sync).
const rootPkgPath = join(repoRoot, 'package.json');
let version = process.argv[2];
if (!version) {
	version = JSON.parse(readFileSync(rootPkgPath, 'utf8')).version;
	console.log(`No version given — re-syncing everything to root package.json (${version}).`);
}
if (!SEMVER.test(version)) {
	console.error(`✗ "${version}" is not a valid semver version (expected e.g. 1.2.0).`);
	process.exit(1);
}

// Each edit is a targeted regex on the raw file text — preserves tabs/spaces and
// leaves every other line byte-for-byte identical. `find` must match exactly once.
const edits = [
	// name, path, matcher, replacement
	['package.json (root)', 'package.json', /("version":\s*)"[^"]*"/, `$1"${version}"`],
	[
		'apps/desktop/package.json',
		'apps/desktop/package.json',
		/("version":\s*)"[^"]*"/,
		`$1"${version}"`,
	],
	[
		'packages/frontend/package.json',
		'packages/frontend/package.json',
		/("version":\s*)"[^"]*"/,
		`$1"${version}"`,
	],
	[
		'packages/types/package.json',
		'packages/types/package.json',
		/("version":\s*)"[^"]*"/,
		`$1"${version}"`,
	],
	// Cargo [package] version: the only `version = "..."` at column 0 (deps are inline).
	[
		'apps/desktop/src-tauri/Cargo.toml',
		'apps/desktop/src-tauri/Cargo.toml',
		/^version = "[^"]*"/m,
		`version = "${version}"`,
	],
	[
		'crates/engine/Cargo.toml',
		'crates/engine/Cargo.toml',
		/^version = "[^"]*"/m,
		`version = "${version}"`,
	],
	// PKGBUILD drives the release download URL, so it must match the tag.
	['packaging/arch/PKGBUILD', 'packaging/arch/PKGBUILD', /^pkgver=.*/m, `pkgver=${version}`],
	// Keep Cargo.lock's two workspace-crate entries in step so `cargo build` is a no-op.
	[
		'Cargo.lock (pea-desktop)',
		'Cargo.lock',
		/(name = "pea-desktop"\nversion = )"[^"]*"/,
		`$1"${version}"`,
	],
	[
		'Cargo.lock (pea-engine)',
		'Cargo.lock',
		/(name = "pea-engine"\nversion = )"[^"]*"/,
		`$1"${version}"`,
	],
];

let failed = false;
for (const [label, rel, find, replace] of edits) {
	const path = join(repoRoot, rel);
	const before = readFileSync(path, 'utf8');
	const matches = before.match(
		new RegExp(find, find.flags.includes('g') ? find.flags : find.flags + 'g')
	);
	if (!matches || matches.length !== 1) {
		console.error(
			`✗ ${label}: expected exactly one version match, found ${matches ? matches.length : 0} — not touched.`
		);
		failed = true;
		continue;
	}
	const after = before.replace(find, replace);
	if (after !== before) writeFileSync(path, after);
	console.log(`✓ ${label} → ${version}`);
}

if (failed) {
	console.error(
		'\nSome files were not updated — review the errors above and fix them before releasing.'
	);
	process.exit(1);
}

console.log(
	`\nAll set to ${version}. tauri.conf.json and the frontend read root package.json, so they update automatically.`
);
console.log(`Next: verify locally, then \`git tag v${version} && git push --tags\` to release.`);
