interface ServerWasm {
	canonicalize_project(files_json: string): string;
	publish_config_facts(text: string): string;
	analyze_project(files_json: string, entries_json: string): string;
}

let loaded: ServerWasm | null = null;
let loading: Promise<ServerWasm> | null = null;

async function initFromBytes(bytes: Uint8Array): Promise<ServerWasm> {
	const glue = (await import('../wasm/wasm_playground.js')) as {
		default: (init: { module_or_path: Uint8Array }) => Promise<unknown>;
		canonicalize_project: (files_json: string) => string;
		publish_config_facts: (text: string) => string;
		analyze_project: (files_json: string, entries_json: string) => string;
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
}

export interface TreeAnalysis {
	capabilities: string[];
	perFile: Record<string, { capabilities: string[]; imports: string[] }>;
	unreachable: string[];
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
		dependencies: (out['dependencies'] ?? {}) as Record<string, string>
	};
}

export async function analyzeTree(
	files: Record<string, string>,
	entries: { main: string; bins: Record<string, string> }
): Promise<TreeAnalysis> {
	const wasm = await serverWasm();
	const out = parseWasmOut(
		wasm.analyze_project(JSON.stringify(files), JSON.stringify(entries)),
		'analyze_project'
	);
	return {
		capabilities: (out['capabilities'] ?? []) as string[],
		perFile: (out['perFile'] ?? {}) as TreeAnalysis['perFile'],
		unreachable: (out['unreachable'] ?? []) as string[]
	};
}
