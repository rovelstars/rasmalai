import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join, dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import type { PackageDetail, VersionDoc } from './db.js';
import { parseFrontmatter } from '$lib/docs/markdown';
import { normalizeGuideEntry } from './guides.js';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..', '..', '..');
const repo = join(root, '..');
const outDir = join(root, '.std-doc');
const apiPath = join(outDir, 'api.json');
const stdDir = join(repo, 'compiler', 'stdlib', 'src');
const SKIP_DIRS = new Set(['target', '.rnx-cache', 'node_modules', '.git', 'wrappers', 'tests']);

function stdlibMtime(): number {
	let newest = 0;
	const walk = (dir: string): void => {
		for (const e of readdirSync(dir, { withFileTypes: true })) {
			if (e.name.startsWith('.') || SKIP_DIRS.has(e.name)) continue;
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

export interface LocalStdEntry {
	name: string;
	size: number;
	dir: boolean;
	chunks: string[];
}

function sha256HexBytes(bytes: Uint8Array): string {
	return createHash('sha256').update(bytes).digest('hex');
}

// Local-dev file index for an @std module, served when the database has
// no row (no Turso binding, or a version never published). One chunk per
// file: the browser concatenates chunk bytes in order, so a single hash
// per file reassembles exactly.
export function localStdEntries(full: string): LocalStdEntry[] | null {
	if (!full.startsWith('@std/')) return null;
	const base = join(stdDir, full.slice('@std/'.length));
	let root: string;
	try {
		root = existsSync(base) ? base : '';
		if (!root) return null;
	} catch {
		return null;
	}
	const out: LocalStdEntry[] = [];
	const walk = (dir: string, rel: string): void => {
		let names: string[];
		try {
			names = readdirSync(dir);
		} catch {
			return;
		}
		for (const name of names) {
			if (name.startsWith('.') || SKIP_DIRS.has(name)) continue;
			const disk = join(dir, name);
			const key = rel ? `${rel}/${name}` : name;
			let isDir = false;
			try {
				isDir = statSync(disk).isDirectory();
			} catch {
				continue;
			}
			if (isDir) {
				out.push({ name: key, size: 0, dir: true, chunks: [] });
				walk(disk, key);
			} else {
				let bytes: Uint8Array;
				try {
					bytes = readFileSync(disk);
				} catch {
					continue;
				}
				// The registry layout puts the module entry under src/. The
				// checkout stores it at <module>/lib.rnx, so present the same
				// path the playground asks for.
				const name = key === 'lib.rnx' ? 'src/lib.rnx' : key;
				out.push({ name, size: bytes.length, dir: false, chunks: [sha256HexBytes(bytes)] });
			}
		}
	};
	walk(root, '');
	if (!out.some((e) => e.dir && e.name === 'src') && out.some((e) => e.name === 'src/lib.rnx')) {
		out.push({ name: 'src', size: 0, dir: true, chunks: [] });
	}
	out.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
	return out;
}

export function localStdChunk(full: string, hash: string): Uint8Array | null {
	if (!/^[0-9a-f]{64}$/.test(hash) || !full.startsWith('@std/')) return null;
	const entries = localStdEntries(full);
	if (!entries) return null;
	for (const e of entries) {
		if (e.dir || !e.chunks.includes(hash)) continue;
		const rel = e.name === 'src/lib.rnx' ? 'lib.rnx' : e.name;
		let bytes: Uint8Array;
		try {
			bytes = readFileSync(join(stdDir, full.slice('@std/'.length), rel));
		} catch {
			return null;
		}
		return sha256HexBytes(bytes) === hash ? bytes : null;
	}
	return null;
}

// Local-dev read-through of the rnx content-addressed store
// (<cache>/store/cas/<aa>/<rest>, seeded by `rnx download`). Covers every
// package whose bytes are on this machine, with no network involved.
export function diskStoreChunk(hash: string): Uint8Array | null {
	if (!/^[0-9a-f]{64}$/.test(hash)) return null;
	const roots: string[] = [];
	const homeCache = process.env['RNX_CACHE_HOME'];
	if (homeCache) roots.push(homeCache);
	const xdg = process.env['XDG_CACHE_HOME'];
	if (xdg) roots.push(join(xdg, 'rnx'));
	const home = process.env['HOME'] ?? process.env['USERPROFILE'];
	if (home) roots.push(join(home, '.cache', 'rnx'));
	for (const root of roots) {
		const disk = join(root, 'store', 'cas', hash.slice(0, 2), hash.slice(2));
		let bytes: Uint8Array;
		try {
			bytes = readFileSync(disk);
		} catch {
			continue;
		}
		if (bytes.length === 0 || bytes.length > 4 * 1024 * 1024) continue;
		if (sha256HexBytes(bytes) === hash) return bytes;
	}
	return null;
}

function localGuides(name: string): string {
	const docs = join(stdDir, name, 'docs');
	if (!existsSync(docs)) return '[]';
	const found: string[] = [];
	const walk = (dir: string): void => {
		let names: string[];
		try {
			names = readdirSync(dir);
		} catch {
			return;
		}
		for (const entry of names) {
			if (entry.startsWith('.')) continue;
			const path = join(dir, entry);
			let isDir = false;
			try {
				isDir = statSync(path).isDirectory();
			} catch {
				continue;
			}
			if (isDir) walk(path);
			else if (entry.endsWith('.md')) found.push(path);
		}
	};
	walk(docs);
	const guides: unknown[] = [];
	for (const path of found.sort()) {
		const rel = relative(docs, path).replace(/\\/g, '/').replace(/\.md$/, '');
		let raw: string;
		try {
			raw = readFileSync(path, 'utf8');
		} catch {
			continue;
		}
		const { meta, body } = parseFrontmatter(raw);
		const heading = body.match(/^#\s+(.+)$/m)?.[1]?.trim() ?? '';
		const stem = rel.split('/').pop() ?? rel;
		const title = meta.title.trim() || heading || stem.replace(/[-_]/g, ' ');
		const entry = normalizeGuideEntry({
			slug: rel,
			title,
			source: body,
			description: meta.description,
			section: meta.section ?? null,
			order: meta.order ?? null
		});
		if (entry.ok) guides.push(entry.entry);
	}
	return JSON.stringify(guides);
}

function packageDescription(name: string, mod: Record<string, unknown> | null): string {
	const overview = join(stdDir, name, 'docs', 'overview.md');
	try {
		const text = readFileSync(overview, 'utf8');
		const { meta, body } = parseFrontmatter(text);
		if (meta.description.trim()) return meta.description.trim();
		const paragraph = body
			.split(/\n\s*\n/)
			.map((p) => p.trim())
			.find((p) => p && !p.startsWith('#') && !p.startsWith('```'));
		if (paragraph) return paragraph;
	} catch {
		/* fall through to the API description */
	}
	const docs = (mod?.['docs'] ?? {}) as Record<string, unknown>;
	const description = String(docs['description'] ?? '').split('\n')[0].trim();
	return description || `@std/${name} standard library module`;
}

function packageReadme(name: string, full: string, version: string): string {
	try {
		const text = readFileSync(join(stdDir, name, 'README.md'), 'utf8').trim();
		if (text) return text + '\n';
	} catch {
		/* fall through to the generic readme */
	}
	return `# ${full}\n\nStandard library module, version ${version}.\n`;
}

function packageVersion(name: string): string | null {
	try {
		const text = readFileSync(join(stdDir, name, 'Project.config'), 'utf8');
		return text.match(/^\s*version:\s*["']([^"']+)["']/m)?.[1] ?? null;
	} catch {
		return null;
	}
}

export function localStdPackage(full: string): { pkg: PackageDetail; doc: VersionDoc } | null {
	if (!full.startsWith('@std/')) return null;
	const name = full.slice('@std/'.length);
	const base = join(stdDir, name);
	if (!existsSync(base)) return null;
	const snap = localStdSnapshot();
	const mod = snap?.modules.find((m) => String(m['name']) === name) ?? null;
	const description = packageDescription(name, mod);
	const version = packageVersion(name) ?? snap?.version ?? cliVersion();
	const now = Math.floor(Date.now() / 1000);
	return {
		pkg: {
			name: full,
			description,
			author: 'Rovel Stars',
			repository: 'https://github.com/rovelstars/rasmalai',
			license: 'MIT OR Apache-2.0',
			stars: 0,
			tags: ['stdlib'],
			keywords: ['stdlib'],
			updatedAt: now,
			latest: version,
			versionCount: 1,
			dependents: 0,
			dependencies: [],
			createdAt: now,
			versions: [{ version, checksum: '', status: 'live', createdAt: now }]
		},
		doc: {
			version,
			readme: packageReadme(name, full, version),
			docJson: mod ? JSON.stringify({ modules: [mod] }) : '{"modules":[]}',
			guides: localGuides(name),
			permissions: []
		}
	};
}
