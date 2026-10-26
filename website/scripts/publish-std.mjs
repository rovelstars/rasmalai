#!/usr/bin/env node
// Publishes every @std/* module as a full registry package through the
// protocol-0 source-tree frame (the server canonicalizes, redacts, chunks
// and analyses the upload), so chunks-fetch, jsdoc, and the code browser
// work for stdlib. Idempotent: already-published versions come back 409
// and are skipped. Re-run after every release; new versions publish, old
// ones skip.
//
//   RNX_BIN=/path/to/rnx SITE_URL=https://... PUBLISH_TOKEN=... node scripts/publish-std.mjs
// RNX_BIN defaults to `cargo run -q -p cli` from the repo root.
import { readFileSync, readdirSync, mkdirSync, rmSync, cpSync, writeFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(root, '..');
const stdDir = join(repo, 'compiler', 'stdlib', 'src');
const stage = join(root, '.std-publish');

const SITE = (process.env['SITE_URL'] ?? 'https://rasmalai.rovelstars.com').replace(/\/$/, '');
const TOKEN = process.env['PUBLISH_TOKEN'];
if (!TOKEN) {
	console.error('publish-std: missing PUBLISH_TOKEN');
	process.exit(1);
}
const rnxBin = process.env['RNX_BIN'];
const rnx = (args, cwd) => {
	if (rnxBin) return execFileSync(rnxBin, args, { cwd, stdio: 'pipe' });
	return execFileSync('cargo', ['run', '-q', '-p', 'cli', '--', ...args], { cwd: repo, stdio: 'pipe' });
};

const cliVersion = readFileSync(join(repo, 'compiler', 'cli', 'Cargo.toml'), 'utf8').match(
	/^version = "(.*)"$/m
)?.[1];
if (!cliVersion) throw new Error('cannot read cli version');
// The token travels in the environment, never in argv: exec failures
// embed the command line, and argv would leak the secret into logs.
process.env['RNX_TOKEN'] = TOKEN;

const modules = [];
for (const e of readdirSync(stdDir, { withFileTypes: true })) {
	if (e.isDirectory() && existsSync(join(stdDir, e.name, 'lib.rnx'))) modules.push(e.name);
}
modules.sort();
// net/http.rnx is a submodule file, not a package: it ships inside @std/net.
const SUBMODULES = { net: ['http.rnx'] };

// Declared permissions per std module, mirroring the server usage scan
// (website/src/lib/server/permissions.ts) over minified sources. Only
// modules whose shipped code observably uses a capability sink declare
// one; everything else publishes with no permissions key (pure).
const PERMISSIONS = {
	fs: [{ perm: 'unsafe:raw_memory', reason: 'mmap is backed by a raw address' }],
	io: [
		'term:read',
		'term:write',
		{ perm: 'env:read:NO_COLOR', reason: 'color detection reads NO_COLOR' },
		{ perm: 'env:read:COLORTERM', reason: 'color detection reads COLORTERM' },
		{ perm: 'env:read:TERM', reason: 'color detection reads TERM' }
	],
	process: [{ perm: 'env:read:*', reason: 'flattenEnv reads caller-provided keys' }]
};

function renderPermissions(name) {
	const decls = PERMISSIONS[name];
	if (!decls) return '';
	const items = decls.map((d) =>
		typeof d === 'string' ? JSON.stringify(d) : `{ perm: ${JSON.stringify(d.perm)}, reason: ${JSON.stringify(d.reason)} }`
	);
	return `,\n    permissions: [${items.join(', ')}]`;
}

const moduleBlurb = (name) => {
	const text = readFileSync(join(stdDir, name, 'lib.rnx'), 'utf8');
	const lines = [];
	for (const line of text.split('\n')) {
		if (!line.startsWith('//!')) break;
		lines.push(line.replace(/^\/\/! ?/, ''));
	}
	while (lines.length > 0 && lines[0].trim() === '') lines.shift();
	const para = [];
	for (const line of lines) {
		if (line.trim() === '') break;
		para.push(line);
	}
	return para.join('\n');
};

rmSync(stage, { recursive: true, force: true });
let published = 0;
let skipped = 0;
// Passes repeat until everything lands: a module whose @std/* dep is not
// published yet fails with "is not preloaded" and retries on the next
// pass once its dep exists. A pass that makes no progress while modules
// remain means a real error, which throws instead of looping forever.
const isMissingDep = (text) => text.includes('is not preloaded');
let pending = [...modules];
let lastError = null;
while (pending.length > 0) {
	const next = [];
	let progressed = false;
	for (const name of pending) {
		const full = `@std/${name}`;
		const dir = join(stage, name.replace('/', '_'));
		mkdirSync(join(dir, 'src'), { recursive: true });
		cpSync(join(stdDir, name, 'lib.rnx'), join(dir, 'src', 'lib.rnx'));
		const docsDir = join(stdDir, name, 'docs');
		if (existsSync(docsDir)) cpSync(docsDir, join(dir, 'docs'), { recursive: true });
		for (const sub of SUBMODULES[name] ?? []) {
			cpSync(join(stdDir, name, sub), join(dir, 'src', sub));
		}
		if (!existsSync(join(dir, 'Project.config'))) {
			writeFileSync(join(dir, 'README.md'), `# ${full}\n\n${moduleBlurb(name)}\n`);
			cpSync(join(repo, 'LICENSE'), join(dir, 'LICENSE'));
			writeFileSync(
				join(dir, 'Project.config'),
				`export default {\n    project: {\n        name: "${full}",\n        version: "${cliVersion}",\n        description: ${JSON.stringify(moduleBlurb(name))}\n    },\n    entries: {\n        main: "src/lib.rnx",\n        docs: "docs"\n    }${renderPermissions(name)}\n}\n`
			);
			rnx(['fmt', join(dir, 'Project.config')], dir);
			for (const f of [join(dir, 'src', 'lib.rnx'), ...(SUBMODULES[name] ?? []).map((s) => join(dir, 'src', s))]) {
				try {
					rnx(['fmt', '--check', f], dir);
				} catch {
					throw new Error(
						`publish-std: staged source ${f} is not fmt-clean; run \`rnx fmt\` on compiler/stdlib and recommit`
					);
				}
			}
		}
		const outDir = join(dir, 'dist');
		mkdirSync(outDir, { recursive: true });
		try {
			rnx(['publish', '--registry', SITE], dir);
			published++;
			progressed = true;
			console.log(`publish-std: published ${full}@${cliVersion}`);
		} catch (e) {
			const text = String(e.stdout ?? '') + String(e.stderr ?? '') + String(e.message ?? '');
			if (text.includes('409') || text.includes('already published')) {
				skipped++;
				progressed = true;
				console.log(`publish-std: skip ${full}@${cliVersion} (exists)`);
			} else if (isMissingDep(text)) {
				lastError = new Error(`publish-std: ${full} waiting on an unpublished dep`);
				next.push(name);
				console.log(`publish-std: defer ${full}@${cliVersion} (dep not published yet)`);
			} else {
				throw e;
			}
		}
	}
	if (!progressed) throw lastError ?? new Error('publish-std: stall with modules remaining');
	pending = next;
}
rmSync(stage, { recursive: true, force: true });
console.log(`publish-std: ${published} published, ${skipped} skipped, ${modules.length} modules`);
