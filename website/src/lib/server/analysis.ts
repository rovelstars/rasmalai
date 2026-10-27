import { getPublishedPackageSources } from './db.js';

interface ServerWasm {
	canonicalize_project(files_json: string): string;
	publish_config_facts(text: string): string;
	analyze_project(files_json: string, entries_json: string): string;
	std_provide(name: string, source: string): string;
}

let loaded: ServerWasm | null = null;
let loading: Promise<ServerWasm> | null = null;

async function initFromBytes(bytes: Uint8Array): Promise<ServerWasm> {
	const glue = (await import('../wasm/wasm_playground.js')) as {
		default: (init: { module_or_path: Uint8Array }) => Promise<unknown>;
		canonicalize_project: (files_json: string) => string;
		publish_config_facts: (text: string) => string;
		analyze_project: (files_json: string, entries_json: string) => string;
		std_provide: (name: string, source: string) => string;
	};
	await glue.default({ module_or_path: bytes });
	return glue;
}

async function initFromModule(module: WebAssembly.Module): Promise<ServerWasm> {
	const glue = (await import('../wasm/wasm_playground.js')) as {
		default: (init: { module_or_path: WebAssembly.Module }) => Promise<unknown>;
		canonicalize_project: (files_json: string) => string;
		publish_config_facts: (text: string) => string;
		analyze_project: (files_json: string, entries_json: string) => string;
		std_provide: (name: string, source: string) => string;
	};
	await glue.default({ module_or_path: module });
	return glue;
}

async function readVendoredWasm(): Promise<Uint8Array | null> {
	try {
		const path = (await import('node:path')) as unknown as {
			join: (...parts: string[]) => string;
			dirname: (p: string) => string;
		};
		const fs = (await import('node:fs')) as unknown as {
			existsSync: (p: string) => boolean;
			readFileSync: (p: string) => Uint8Array;
		};
		const url = (await import('node:url')) as unknown as {
			fileURLToPath: (u: string) => string;
		};
		const here = url.fileURLToPath(import.meta.url);
		const disk = path.join(path.dirname(here), '..', 'wasm', 'wasm_playground_bg.wasm');
		if (!fs.existsSync(disk)) return null;
		return new Uint8Array(fs.readFileSync(disk));
	} catch {
		return null;
	}
}

// The registry server canonicalizes and analyses with the same Rust code
// the client packs with, compiled to WASM. Under Node (tests, local dev)
// the vendored bytes are read from disk; in the Worker they arrive as the
// deploy-time precompiled module below. workerd forbids compiling Wasm
// from bytes at runtime, so fetching the binary and instantiating it in
// the Worker can never work: the static import must survive the build for
// the deploy bundler to precompile, never a URL plus fetch plus compile.
async function loadPrecompiledModule(): Promise<WebAssembly.Module> {
	let module: unknown;
	try {
		module = (await import('./wasm-module')).default;
	} catch {
		throw new Error('analysis unavailable (wasm module could not be loaded)');
	}
	if (!(module instanceof WebAssembly.Module)) {
		throw new Error('analysis unavailable (wasm module is not precompiled)');
	}
	return module;
}

export async function serverWasm(): Promise<ServerWasm> {
	if (loaded) return loaded;
	if (loading) return loading;
	loading = (async () => {
		const bytes = await readVendoredWasm();
		loaded = bytes ? await initFromBytes(bytes) : await initFromModule(await loadPrecompiledModule());
		return loaded;
	})();
	try {
		return await loading;
	} catch (e) {
		loading = null;
		throw e;
	}
}

export interface CanonicalTree {
	files: Record<string, string>;
}

export interface ConfigFacts {
	name: string;
	version: string;
	redacted: string;
	entries: { main: string; lib: string | null; docs: string | null; bins: Record<string, string> };
	permissions: Array<{ perm: string; reason: string | null }>;
	dependencies: Record<string, string>;
	artifacts: string | null;
}

export interface ReportOnlyVerdicts {
	s103: { count: number; files: Record<string, string[]> };
	registries: { keys: string[] };
}

export interface TreeAnalysis {
	capabilities: string[];
	perFile: Record<string, { capabilities: string[]; imports: string[] }>;
	unreachable: string[];
	reportOnly: ReportOnlyVerdicts;
}

function parseReportOnly(out: Record<string, unknown>): ReportOnlyVerdicts {
	const fallback: ReportOnlyVerdicts = { s103: { count: 0, files: {} }, registries: { keys: [] } };
	const raw = out['reportOnly'];
	if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return fallback;
	const node = raw as Record<string, unknown>;
	const s103 = node['s103'];
	const registries = node['registries'];
	const files: Record<string, string[]> = {};
	if (s103 && typeof s103 === 'object' && !Array.isArray(s103)) {
		const table = (s103 as Record<string, unknown>)['files'];
		if (table && typeof table === 'object' && !Array.isArray(table)) {
			for (const [path, names] of Object.entries(table as Record<string, unknown>)) {
				if (Array.isArray(names)) {
					files[path] = names.filter((n): n is string => typeof n === 'string');
				}
			}
		}
	}
	const count =
		s103 && typeof s103 === 'object' && !Array.isArray(s103) && typeof (s103 as Record<string, unknown>)['count'] === 'number'
			? ((s103 as Record<string, unknown>)['count'] as number)
			: 0;
	const keys =
		registries && typeof registries === 'object' && !Array.isArray(registries) && Array.isArray((registries as Record<string, unknown>)['keys'])
			? (((registries as Record<string, unknown>)['keys'] as unknown[]).filter((k): k is string => typeof k === 'string'))
			: [];
	return { s103: { count, files }, registries: { keys } };
}

function parseWasmOut(raw: string, entry: string): Record<string, unknown> {
	let out: Record<string, unknown>;
	try {
		out = JSON.parse(raw) as Record<string, unknown>;
	} catch {
		throw new Error(`${entry} returned unparseable output`);
	}
	if (!out['ok']) throw new Error(String(out['error'] ?? `${entry} failed`));
	return out;
}

export async function canonicalizeTree(files: Record<string, string>): Promise<CanonicalTree> {
	const wasm = await serverWasm();
	const out = parseWasmOut(wasm.canonicalize_project(JSON.stringify(files)), 'canonicalize_project');
	return { files: out['files'] as Record<string, string> };
}

export async function configFacts(text: string): Promise<ConfigFacts> {
	const wasm = await serverWasm();
	const out = parseWasmOut(wasm.publish_config_facts(text), 'publish_config_facts');
	const entries = out['entries'] as ConfigFacts['entries'];
	const rawPermissions = (out['permissions'] ?? []) as Array<string | { perm: string; reason?: string }>;
	return {
		name: String(out['name'] ?? ''),
		version: String(out['version'] ?? ''),
		redacted: String(out['redacted'] ?? ''),
		entries,
		permissions: rawPermissions.map((p) =>
			typeof p === 'string' ? { perm: p, reason: null } : { perm: p.perm, reason: p.reason ?? null }
		),
		dependencies: (out['dependencies'] ?? {}) as Record<string, string>,
		artifacts: typeof out['artifacts'] === 'string' ? (out['artifacts'] as string) : null
	};
}

export async function analyzeTree(
	env: Record<string, string | undefined>,
	files: Record<string, string>,
	entries: { main: string; bins: Record<string, string>; packageName?: string; configText?: string }
): Promise<TreeAnalysis> {
	const wasm = await serverWasm();
	// @std/* imports resolve from provided sources, mirroring the
	// playground two-phase flow. Deps come from R2 (hash-verified), never
	// the client. Provide-then-analyze has no await between the two, so a
	// round is atomic; sources are re-provided every publish because the
	// isolate VFS is unversioned and must never serve stale dep bytes.
	const provided = new Set<string>();
	for (let round = 0; round < 10; round++) {
		try {
			const out = parseWasmOut(
				wasm.analyze_project(JSON.stringify(files), JSON.stringify(entries)),
				'analyze_project'
			);
			return {
				capabilities: (out['capabilities'] ?? []) as string[],
				perFile: (out['perFile'] ?? {}) as TreeAnalysis['perFile'],
				unreachable: (out['unreachable'] ?? []) as string[],
				reportOnly: parseReportOnly(out)
			};
		} catch (e) {
			const msg = e instanceof Error ? e.message : String(e);
			const missing = /standard library module `(@std\/[^`]+)` is not preloaded/.exec(msg)?.[1];
			if (!missing) throw e;
			const rest = missing.replace(/^@std\//, '');
			if (provided.has(rest)) throw e;
			provided.add(rest);
			const dep = await getPublishedPackageSources(env, `@std/${rest.split('/')[0]}`);
			if (!dep) throw new Error(`${msg} (hint: @std/${rest.split('/')[0]} is not published; publish it first)`);
			for (const [name, source] of Object.entries(dep.files)) {
				parseWasmOut(wasm.std_provide(name, source), 'std_provide');
				provided.add(name);
			}
		}
	}
	throw new Error('analysis unavailable (dependency resolution did not converge)');
}
