import type { DocModule } from '$lib/docs/api';
import { dev } from '$app/environment';
import { fetchDataFile } from '$lib/docs/data-url';

// api.json is generated at build time (prebuild) into static/data and
// served same-origin. This export is the legacy unversioned fallback;
// resolution prefers the deploy-versioned copy via /data/version.json
// (see data-url.ts), so no build carries a snapshot from the repo.
// Point a deploy somewhere else by editing the fallback here.
export const apiUrl = '/data/api.json';

interface ApiSnapshot {
	modules: DocModule[];
}

let cached: ApiSnapshot | null = null;
let inflight: Promise<ApiSnapshot | null> | null = null;

export function ensureApi(fetchFn: typeof fetch = fetch): Promise<ApiSnapshot | null> {
	if (cached) return Promise.resolve(cached);
	inflight ??= (async () => {
		try {
			if (dev) {
				try {
					const local = await fetchFn('/api/std-local');
					if (local.ok) {
						const data = (await local.json()) as ApiSnapshot;
						if (data && Array.isArray(data.modules)) {
							cached = data;
							return cached;
						}
					}
					console.warn(`rnx docs: local snapshot unusable (${local.status}), falling back to static api.json`);
				} catch {
					console.warn('rnx docs: /api/std-local unreachable, falling back to static api.json');
					/* fall through to the static files */
				}
			}
			const res = await fetchDataFile(fetchFn, 'api.json');
			if (!res) return null;
			const data = (await res.json()) as ApiSnapshot;
			if (!data || !Array.isArray(data.modules)) return null;
			cached = data;
			return cached;
		} catch {
			return null;
		} finally {
			inflight = null;
		}
	})();
	return inflight;
}

export function hasApi(): boolean {
	return cached !== null;
}

export const ENGINE_VERSION = '0.1.0';
export const STD_LICENSE = 'MIT';

export interface StdModuleMeta {
	name: string;
	tagline: string;
	primaryExport: string;
	whenToUse: string;
	capabilities: string[];
	quickstart?: string;
}

const QUICKSTART_BYTES = `import { ByteBuffer } from "@std/bytes";


let buf = ByteBuffer.allocate(16);
buf.writeInt32BE(0, 0x01020304);
print(buf.readInt32BE(0));
print(buf.writeString(4, "Rasmalai"));
print(buf.readString(4, 8));
`;

const QUICKSTART_OS = `import { OS } from "@std/os";


print(OS.platform(), OS.arch());
print(OS.cpuCount());
print(OS.tmpdir());
`;

const QUICKSTART_PROCESS = `import { Process } from "@std/process";


let out = Process.run("echo", ["hello"]);
print(out.exitCode);
print(out.stdoutText().contains("hello"));
`;

const QUICKSTART_TESTING = `import { blackBox } from "@std/testing";


let x = blackBox(40 + 2);
print(x);
`;

const QUICKSTART_WEB = `import { URL, Headers, statusCode, HttpStatus } from "@std/web";


let u = new URL("https://example.com:8080/a/b?x=1#frag");
print(u.href());
print(u.hostname(), u.port(), u.pathname());
let h = new Headers();
h.set("Content-Type", "text/plain");
print(h.get("content-type").unwrap());
print(statusCode(HttpStatus.NotFound));
`;

const QUICKSTART_JSON = `import { JSON } from "@std/json";


print(JSON.stringify(JSON.parse("12345")));
print(JSON.stringify(JSON.parse("\"hello world\"")));
let obj = JSON.parseObject("\{\"count\": 41}");
print(obj.get("count"));
`;

const QUICKSTART_NET = `import { TcpListener } from "@std/net";


let listener = TcpListener.bind("127.0.0.1", 0);
print(listener.port() > 0);
listener.close();
`;

const QUICKSTART_PRELUDE = `print("no import needed");
let found = [1, 2, 3].find((n) => n > 1);
assert("abc".contains("b"), "prelude");
print(found.unwrap(), typeOf(found));
`;

export const STD_MODULES: StdModuleMeta[] = [
	{
		name: 'prelude',
		tagline: 'Implicit scope: foundation types, root intrinsics, and Array/String extensions.',
		primaryExport: 'Option',
		whenToUse:
			'The prelude is already imported everywhere: every file sees Option, Result, Array, Map, print, assert, and the Array/String extension methods with no import. Reference @std/prelude explicitly only to disambiguate a shadowed name.',
		capabilities: ['Int/Float/Bool/String/Array/Map/Set', 'print/assert/typeOf intrinsics', 'Option/Result/Iterator', 'Array/String extensions'],
		quickstart: QUICKSTART_PRELUDE
	},
	{
		name: 'bytes',
		tagline: 'Binary allocation, endian-aware reading/writing, byte slicing.',
		primaryExport: 'ByteBuffer',
		whenToUse:
			'Reach for @std/bytes when a program talks to the outside world in binary: wire protocols, file headers, or any format where byte order matters. Text stays in String; everything else goes through a ByteBuffer.',
		capabilities: ['Fixed-size raw buffers', 'Explicit-endian int/float lanes', 'Zero-copy slicing'],
		quickstart: QUICKSTART_BYTES
	},
	{
		name: 'collections',
		tagline: 'Generic Map<K, V>, Set<T>, and collection utilities.',
		primaryExport: 'Map',
		whenToUse:
			'Reach for @std/collections when keys need lookup or members need deduplication. Map preserves insertion order for keys() and values(); Set is built on top of it with the same guarantee.',
		capabilities: ['Insertion-ordered Map', 'Distinct-member Set', 'for..in iteration']
	},
	{
		name: 'env',
		tagline: 'Environment variables, command-line arguments, working directory.',
		primaryExport: 'Env',
		whenToUse:
			'Reach for @std/env when a program reads its configuration from the outside: CLI arguments, environment variables, or the current directory. Process.run covers spawning; Env covers reading.',
		capabilities: ['argv access', 'Environment get/set', 'Working directory']
	},
	{
		name: 'fs',
		tagline: 'Filesystem one-shots, file handles, and path helpers.',
		primaryExport: 'fs',
		whenToUse:
			'Reach for @std/fs when a program persists anything. Bare names are sync and blocking one-shots (readText/writeText/copy/remove) with *Async twins on the process-wide pool; File handles carry a cursor with seek/tell plus advisory locks; Path covers joins and queries; mmap maps files into unsafe memory.',
		capabilities: ['One-shot read/write plus *Async twins', 'File handles with cursor and locks', 'Path helpers', 'Mmap address mapping']
	},
	{
		name: 'math',
		tagline: 'Scalar helpers, Vec2, and numeric utilities.',
		primaryExport: 'Math',
		whenToUse:
			'Reach for @std/math for scalar numerics (sqrt, trig, floor/ceil) and the Vec2 value type for 2D geometry. For 4-lane vector throughput, step up to @std/simd.',
		capabilities: ['IEEE-754 intrinsics', 'Vec2 geometry', 'Float helpers']
	},
	{
		name: 'os',
		tagline: 'System architecture, platform constants, CPU counts, uptime.',
		primaryExport: 'OS',
		whenToUse:
			'Reach for @std/os when a program adapts to where it runs: platform branches, CPU-count-sized pools, temp directories, or uptime telemetry. Pure queries, no handles to close.',
		capabilities: ['Platform/arch queries', 'CPU and uptime', 'Standard directories'],
		quickstart: QUICKSTART_OS
	},
	{
		name: 'process',
		tagline: 'Process lifecycle, child spawning, pipe I/O, signals.',
		primaryExport: 'Process',
		whenToUse:
			'Reach for @std/process to control the host (args, env, cwd, exit) or to run children: Process.run captures a command to completion, while spawn plus ByteBuffer pipes drives interactive children. Always wait() what you spawn.',
		capabilities: ['Host control', 'spawn/run lifecycles', 'ByteBuffer pipes'],
		quickstart: QUICKSTART_PROCESS
	},
	{
		name: 'random',
		tagline: 'Deterministic seeded streams and OS entropy.',
		primaryExport: 'Rng',
		whenToUse:
			'Reach for @std/random in two shapes: Rng.seeded(n) for deterministic streams that replay bit-identically (simulations, tests), CryptoRng for OS entropy when unpredictability matters.',
		capabilities: ['Seeded Rng streams', 'OS-backed CryptoRng', 'Ints, bools, ranges']
	},
	{
		name: 'simd',
		tagline: '128-bit SIMD vector value types with no heap allocation.',
		primaryExport: 'Vec4f',
		whenToUse:
			'Reach for @std/simd when four floats (or ints) move together: graphics, physics, signal processing. Vec4f lives inline with plain operators — no heap, no ARC traffic, no intrinsics headers.',
		capabilities: ['Vec4f/Vec4i lanes', 'Reductions (dot/min/max)', 'Zero-allocation ops']
	},
	{
		name: 'sync',
		tagline: 'Concurrency primitives: AtomicInt, mutexes, barriers, channels.',
		primaryExport: 'AtomicInt',
		whenToUse:
			'Reach for @std/sync whenever threads share anything. Every primitive is constructed byId so separate threads open the same cell — the only legal cross-thread sharing path in the language.',
		capabilities: ['AtomicInt counters', 'Mutex/RwLock guards', 'Channel/Barrier coordination']
	},
	{
		name: 'testing',
		tagline: 'Benchmark helpers and optimizer fences.',
		primaryExport: 'blackBox',
		whenToUse:
			'Reach for @std/testing inside benchmarks: blackBox keeps the optimizer from deleting the code being measured. Tests themselves live in test fn blocks and run under rnx test.',
		capabilities: ['blackBox fences', 'Benchmark support'],
		quickstart: QUICKSTART_TESTING
	},
	{
		name: 'time',
		tagline: 'Deterministic virtual clocks and monotonic time.',
		primaryExport: 'Clock',
		whenToUse:
			'Reach for @std/time in two shapes: Clock.mono for real monotonic durations, and Clock.virtual plus tick() for deterministic virtual time that tests can drive step by step.',
		capabilities: ['Monotonic clock', 'VirtualClock control', 'Duration math']
	},
	{
		name: 'web',
		tagline: 'Synchronous web primitives: status codes, headers, URLs, query strings.',
		primaryExport: 'URL',
		whenToUse:
			'Reach for @std/web for pure URL and header work with zero I/O: parsing hrefs, resolving relative references, manipulating query strings, or mapping status codes. fetch() and WebSocket stay out until the network RFC lands.',
		capabilities: ['HttpStatus codes', 'Case-insensitive Headers', 'URL parse/resolve', 'URLSearchParams'],
		quickstart: QUICKSTART_WEB
	},
	{
		name: 'json',
		tagline: 'JSON parsing, stringification, and object mapping.',
		primaryExport: 'JSON',
		whenToUse:
			'Reach for @std/json at every process boundary that speaks JSON: parsing request bodies and config files with JSON.parse, emitting responses with JSON.stringify, or pulling typed maps out with JSON.parseObject and JSON.asMap.',
		capabilities: ['JSON.parse/stringify', 'parseObject to Map', 'asMap/asArray casts'],
		quickstart: QUICKSTART_JSON
	},
	{
		name: 'net',
		tagline: 'Non-blocking TCP, TLS, and DNS over the background reactor.',
		primaryExport: 'TcpStream',
		whenToUse:
			'Reach for @std/net when a program touches the network directly: TcpStream.connect for clients, TcpListener.bind plus accept for servers, TlsStream for encrypted sessions, Dns.lookup for resolution. Every operation parks in the kernel until ready; nothing spins.',
		capabilities: ['TcpStream connect/read/write', 'TcpListener bind/accept', 'TlsStream sessions', 'Dns.lookup'],
		quickstart: QUICKSTART_NET
	},
	{
		name: 'io',
		tagline: 'Terminal streams, pretty-printing, line input, and raw mode.',
		primaryExport: 'io',
		whenToUse:
			'Reach for @std/io when a program talks to the terminal: write and writeError for color-aware pretty-printing with newlines, writeRaw for exact bytes, read and readLine for input, width and height for layout, colorProfile for the ColorLevel tier, setRawMode for character-at-a-time input. File I/O stays in @std/fs.',
		capabilities: ['Color-aware pretty-printing', 'stdin/stdout/stderr handles', 'Line input and size queries', 'Raw mode toggling']
	}
];

export function stdMeta(name: string): StdModuleMeta {
	const meta = STD_MODULES.find((m) => m.name === name);
	if (!meta) throw new Error(`unknown @std module: ${name}`);
	return meta;
}

export function stdApiModule(name: string): DocModule | null {
	const mod = cached?.modules.find((m) => m.name === name);
	return mod ?? null;
}

export function stdApiModules(): DocModule[] | null {
	return cached?.modules ?? null;
}

export function importSnippet(name: string): string {
	if (name === 'prelude') return '// @std/prelude is implicit: no import required';
	return `import { ${stdMeta(name).primaryExport} } from "@std/${name}";`;
}

export function firstExample(name: string): string | null {
	const mod = stdApiModule(name);
	if (!mod) return null;
	for (const c of mod.classes) {
		for (const meth of c.methods) {
			const ex = meth.docs.tags.find((t) => t.kind === 'example');
			if (ex) return ex.text;
		}
	}
	for (const f of mod.functions) {
		const ex = f.docs.tags.find((t) => t.kind === 'example');
		if (ex) return ex.text;
	}
	return null;
}

export function quickstartFor(name: string): string {
	return stdMeta(name).quickstart ?? firstExample(name) ?? importSnippet(name);
}
