// Declared-permission parsing for the registry.
//
// Capability grammar mirror. Source of truth is the compiler:
// compiler/frontend/src/capabilities.rs (`Capability::from_str`).
// Keep CAPABILITY_DOMAINS and splitPermission in sync when that grammar
// changes. Usage scanning below mirrors the compiler `sink_for` table in
// the same file at domain-head granularity: it is a textual heuristic,
// not the taint analyzer, and the wasm playground exports no analysis
// entry point (check/run/tokenize only), so the server cannot shell out
// to the real pass. Under-declaration is rejected; over-declaration (a
// wider ceiling than the code uses) stays allowed.
import { parseTarEntries } from '$lib/server/chunks';

export const CAPABILITY_DOMAINS = [
	'fs:read',
	'fs:write',
	'fs:delegated',
	'net:http',
	'net:ws',
	'net:delegated',
	'sys:exec',
	'env:read',
	'env:dump',
	'term:write',
	'term:read',
	'term:raw',
	'unsafe:ffi',
	'unsafe:raw_memory'
] as const;

export type CapabilityDomain = (typeof CAPABILITY_DOMAINS)[number];

const BARE_DOMAINS = new Set<string>([
	'fs:delegated',
	'net:delegated',
	'unsafe:ffi',
	'unsafe:raw_memory',
	'term:write',
	'term:read',
	'term:raw',
	'env:dump'
]);

const PARAMETRIC_DOMAINS = new Set<string>([
	'fs:read',
	'fs:write',
	'net:http',
	'net:ws',
	'sys:exec',
	'env:read'
]);

export interface DeclaredPermission {
	domain: string;
	arg: string;
	reason: string | null;
}

export function splitPermission(perm: string): { domain: string; arg: string } {
	if (typeof perm !== 'string' || perm === '') {
		throw new Error(`malformed permission \`${String(perm)}\``);
	}
	if (BARE_DOMAINS.has(perm)) return { domain: perm, arg: '' };
	const first = perm.indexOf(':');
	const second = first < 0 ? -1 : perm.indexOf(':', first + 1);
	if (first <= 0 || second <= first + 1) {
		throw new Error(`malformed permission \`${perm}\``);
	}
	const domain = perm.slice(0, second);
	if (!PARAMETRIC_DOMAINS.has(domain)) {
		throw new Error(`unknown permission \`${perm}\``);
	}
	return { domain, arg: perm.slice(second + 1) };
}

function toDeclared(item: unknown): DeclaredPermission {
	if (typeof item === 'string') {
		const { domain, arg } = splitPermission(item);
		return { domain, arg, reason: null };
	}
	if (typeof item === 'object' && item !== null && !Array.isArray(item)) {
		const table = item as Record<string, unknown>;
		if (typeof table['perm'] !== 'string') {
			throw new Error('permission entries need a string `perm`');
		}
		const { domain, arg } = splitPermission(table['perm']);
		const reason = table['reason'];
		if (reason !== undefined && reason !== null && typeof reason !== 'string') {
			throw new Error('permission `reason` must be a string');
		}
		return { domain, arg, reason: typeof reason === 'string' ? reason : null };
	}
	throw new Error('permissions must be strings or `{ perm, reason }` tables');
}

export function parseManifestPermissions(manifestJson: string): DeclaredPermission[] {
	let manifest: Record<string, unknown>;
	try {
		manifest = JSON.parse(manifestJson) as Record<string, unknown>;
	} catch {
		return [];
	}
	const raw = manifest['permissions'];
	if (raw === undefined) return [];
	if (!Array.isArray(raw)) {
		throw new Error('manifest `permissions` must be an array');
	}
	return raw.map(toDeclared);
}

export type CapabilityHead = 'fs' | 'net' | 'sys' | 'env' | 'term' | 'unsafe';

export interface CapabilityEvidence {
	head: CapabilityHead;
	symbol: string;
	file: string;
}

// Dotted stdlib paths from the compiler `sink_for` table, grouped by the
// domain head a declaration must cover. Bare globals (`fetch`, `print`)
// only count as calls, never as declarations or type names.
const DOTTED_SINKS: Array<{ head: CapabilityHead; names: string[] }> = [
	{
		head: 'fs',
		names: [
			'File.open', 'File.read', 'File.readText', 'File.readBytes', 'File.lines',
			'File.exists', 'File.seek', 'File.tell', 'File.len', 'File.write',
			'File.writeText', 'File.writeBytes', 'File.append', 'File.create',
			'File.remove', 'File.truncate', 'Path.exists', 'Path.isFile',
			'Path.isDir', 'Path.remove', 'fs.stat', 'fs.exists', 'fs.isFile',
			'fs.isDir', 'fs.readDir', 'fs.glob', 'fs.readLink', 'fs.readText',
			'fs.readBytes', 'fs.mkdir', 'fs.mkdirAll', 'fs.remove', 'fs.removeAll',
			'fs.copy', 'fs.move', 'fs.rename', 'fs.truncate', 'fs.chmod',
			'fs.symlink', 'fs.fsync', 'fs.writeText', 'fs.writeBytes', 'fs.mmap'
		]
	},
	{ head: 'sys', names: ['Process.spawn', 'Process.run'] },
	{
		head: 'env',
		names: ['Env.get', 'Env.has', 'Env.all', 'Env.allEnv', 'env.get', 'env.has', 'env.all']
	},
	{
		head: 'term',
		names: [
			'File.fromHandle', 'io.write', 'io.writeRaw', 'io.writeError', 'io.clear',
			'io.stdout', 'io.stderr', 'io.read', 'io.readLine', 'io.stdin',
			'io.isTTY', 'io.width', 'io.height', 'io.colorProfile', 'io.setRawMode'
		]
	}
];

const BARE_SINKS: Array<{ head: CapabilityHead; names: string[] }> = [
	{ head: 'net', names: ['fetch', 'Request', 'WebSocket'] },
	{ head: 'term', names: ['print'] }
];

// Suffix fallback from `sink_for`: any `.writeText`/`.writeBytes` call
// writes to the terminal, any `.readText`/`.readBytes` call reads from it.
const TERM_SUFFIX_RE = /\.\s*(writeText|writeBytes|readText|readBytes)\s*\(/g;

function stripToCode(source: string): string {
	let s = source.replace(/"""[\s\S]*?"""/g, '""');
	s = s.replace(/"(?:[^"\\\n]|\\.)*"/g, '""');
	s = s.replace(/'(?:[^'\\\n]|\\.)*'/g, "''");
	s = s.replace(/\/\*[\s\S]*?\*\//g, ' ');
	s = s.replace(/\/\/.*$/gm, ' ');
	s = s.replace(/\bclass\s+[A-Za-z_][A-Za-z0-9_]*/g, 'class');
	s = s.replace(/\bfn\s+[A-Za-z_][A-Za-z0-9_]*/g, 'fn');
	return s;
}

function escRe(s: string): string {
	return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

export function detectCapabilityUse(
	files: Array<{ name: string; text: string }>
): CapabilityEvidence[] {
	const seen = new Set<string>();
	const out: CapabilityEvidence[] = [];
	const push = (head: CapabilityHead, symbol: string, file: string): void => {
		const key = `${head}:${file}`;
		if (seen.has(key)) return;
		seen.add(key);
		out.push({ head, symbol, file });
	};
	for (const f of files) {
		if (!f.name.endsWith('.rnx')) continue;
		const code = stripToCode(f.text);
		for (const group of DOTTED_SINKS) {
			for (const name of group.names) {
				const re = new RegExp(`(?<![A-Za-z0-9_.])${escRe(name)}\\s*\\(`);
				if (re.test(code)) push(group.head, name, f.name);
			}
		}
		for (const group of BARE_SINKS) {
			for (const name of group.names) {
				const re = new RegExp(`(?<![A-Za-z0-9_.])${escRe(name)}\\s*\\(`);
				if (re.test(code)) push(group.head, name, f.name);
			}
		}
		if (TERM_SUFFIX_RE.test(code)) {
			TERM_SUFFIX_RE.lastIndex = 0;
			const m = /\.\s*(writeText|writeBytes|readText|readBytes)\s*\(/.exec(code);
			push('term', m ? m[1] : 'suffix-call', f.name);
		}
		if (/\bfrom\s+native\b/.test(code)) push('unsafe', 'from native', f.name);
		if (/\bunsafe\b/.test(code)) push('unsafe', 'unsafe', f.name);
	}
	out.sort((a, b) => (a.head < b.head ? -1 : a.head > b.head ? 1 : a.file < b.file ? -1 : 1));
	return out;
}

export function isGzipTarball(bytes: Uint8Array): boolean {
	return bytes.length >= 2 && bytes[0] === 0x1f && bytes[1] === 0x8b;
}

// Bounded inflate for scanning only: gunzips at most SCAN_MAX_TARBALL_BYTES
// of output, then gives up and reports nothing. The tarball itself is
// already capped at 512 KiB on the wire, but a hostile gzip bomb would
// expand far past that in memory; skipping the scan (declarations stored
// unchecked) beats OOMing the worker, and storage caps still apply.
const SCAN_MAX_TARBALL_BYTES = 4 * 1024 * 1024;

async function inflateBounded(bytes: Uint8Array): Promise<Uint8Array | null> {
	try {
		const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new DecompressionStream('gzip'));
		const reader = stream.getReader();
		const parts: Uint8Array[] = [];
		let total = 0;
		for (;;) {
			const { done, value } = await reader.read();
			if (done) break;
			total += value.byteLength;
			if (total > SCAN_MAX_TARBALL_BYTES) {
				await reader.cancel().catch(() => undefined);
				return null;
			}
			parts.push(value);
		}
		const out = new Uint8Array(total);
		let off = 0;
		for (const p of parts) {
			out.set(p, off);
			off += p.byteLength;
		}
		return out;
	} catch {
		return null;
	}
}

export async function scanTarballCapabilities(tarball: Uint8Array): Promise<CapabilityEvidence[]> {
	let bytes = tarball;
	if (isGzipTarball(bytes)) {
		const inflated = await inflateBounded(bytes);
		if (!inflated) return [];
		bytes = inflated;
	}
	const files: Array<{ name: string; text: string }> = [];
	const dec = new TextDecoder();
	for (const e of parseTarEntries(bytes)) {
		if (!e.name.endsWith('.rnx')) continue;
		files.push({ name: e.name, text: dec.decode(e.bytes) });
	}
	return detectCapabilityUse(files);
}

export type CoverageResult = { ok: true } | { ok: false; error: string };

// Every domain head the shipped code observably uses must have at least
// one declared permission under it. Args are opaque (a declaration of
// `fs:read:/data` covers any `fs` use); only the head is compared.
export function checkCapabilityCoverage(
	declared: DeclaredPermission[],
	used: CapabilityEvidence[]
): CoverageResult {
	const heads = new Set(declared.map((d) => d.domain.split(':')[0]));
	const missing: CapabilityEvidence[] = [];
	const seenHead = new Set<string>();
	for (const u of used) {
		if (!heads.has(u.head) && !seenHead.has(u.head)) {
			seenHead.add(u.head);
			missing.push(u);
		}
	}
	if (missing.length === 0) return { ok: true };
	const want = missing.map((m) => `${m.head} (e.g. \`${m.symbol}\` in ${m.file})`).join(', ');
	const have = declared.length > 0 ? declared.map((d) => `\`${d.domain}${d.arg ? `:${d.arg}` : ''}\``).join(', ') : 'nothing';
	return {
		ok: false,
		error:
			`package code uses ${want} but the manifest declares ${have}; ` +
			`add { perm = "...", reason = "..." } entries covering those heads to Project.config permissions`
	};
}
