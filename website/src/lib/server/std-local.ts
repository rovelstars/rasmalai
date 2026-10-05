import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import type { PackageDetail, VersionDoc } from './db.js';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..', '..', '..');
const repo = join(root, '..');
const outDir = join(root, '.std-doc');
const apiPath = join(outDir, 'api.json');
const stdDir = join(repo, 'compiler', 'stdlib', 'src');

function stdlibMtime(): number {
	let newest = 0;
	const walk = (dir: string): void => {
		for (const e of readdirSync(dir, { withFileTypes: true })) {
			const p = join(dir, e.name);
			if (e.isDirectory()) walk(p);
			else if (e.name.endsWith('.rnx')) {
				const m = statSync(p).mtimeMs;
				if (m > newest) newest = m;
			}
		}
	};
	if (existsSync(stdDir)) walk(stdDir);
	return newest;
}

function cliVersion(): string {
	const text = readFileSync(join(repo, 'compiler', 'cli', 'Cargo.toml'), 'utf8');
	return text.match(/^version = "(.*)"$/m)?.[1] ?? '0.0.0';
}

function apiJson(): { version: string; modules: Array<Record<string, unknown>> } | null {
	try {
		if (!existsSync(apiPath)) return null;
		if (statSync(apiPath).mtimeMs < stdlibMtime()) return null;
		return JSON.parse(readFileSync(apiPath, 'utf8')) as {
			version: string;
			modules: Array<Record<string, unknown>>;
		};
	} catch {
		return null;
	}
}

export function localStdSnapshot(): { version: string; modules: Array<Record<string, unknown>> } | null {
	const cached = apiJson();
	if (cached && cached.modules.length > 0) return cached;
	try {
		execFileSync('cargo', ['run', '-q', '-p', 'cli', '--', 'doc', '--json', '--stdlib', '--out-dir', outDir], {
			cwd: repo,
			stdio: 'pipe'
		});
	} catch {
		return null;
	}
	return apiJson();
}

export function localStdPackage(full: string): { pkg: PackageDetail; doc: VersionDoc } | null {
	if (!full.startsWith('@std/')) return null;
	const snap = localStdSnapshot();
	if (!snap) return null;
	const name = full.slice('@std/'.length);
	const mod = snap.modules.find((m) => String(m['name']) === name);
	if (!mod) return null;
	const docs = (mod['docs'] ?? {}) as Record<string, unknown>;
	const description = String(docs['description'] ?? `${full} standard library module`).split('\n')[0];
	const version = snap.version || cliVersion();
	const now = Math.floor(Date.now() / 1000);
	return {
		pkg: {
			name: full,
			description,
			author: 'Rovel Stars',
			repository: 'https://github.com/rovelstars/rasmalai',
			license: 'MIT',
			downloads: 0,
			stars: 0,
			tags: ['stdlib'],
			updatedAt: now,
			latest: version,
			versionCount: 1,
			dependencies: [],
			createdAt: now,
			versions: [{ version, checksum: '', status: 'live', createdAt: now }]
		},
		doc: {
			version,
			readme: `# ${full}\n\nStandard library module, version ${version}.\n`,
			docJson: JSON.stringify({ modules: [mod] })
		}
	};
}
