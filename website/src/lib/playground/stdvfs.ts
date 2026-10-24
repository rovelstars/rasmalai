// Registry-backed stdlib file view for the playground engine.
//
// The wasm engine ships no embedded `@std/*` sources. The main thread
// preloads `@std/prelude` at engine download and lazily fetches every
// other imported module from the same-domain registry (fixed 15_REGISTRY
// chunk paths, never user-supplied URLs), verifies each chunk against its
// sha256, and hands the bytes to the engine over the worker protocol.
// Versioned package URLs are immutable, so fetched modules cache in
// IndexedDB beside the engine bytes forever.

import { CANONICAL_ORIGIN } from '$lib/site';

export interface ChunkEntry {
	name: string;
	size: number;
	dir: boolean;
	chunks: string[];
}

export const STD_DB = 'rnx-engine';
export const STD_STORE = 'std';
const STD_KEY_PREFIX = 'std::';
const MAX_STD_MODULE_BYTES = 1024 * 1024;
const PREFETCH_ROUNDS = 8;

export const PROD_REGISTRY = CANONICAL_ORIGIN;

export function isDevOrigin(origin: string): boolean {
	try {
		const host = new URL(origin).hostname.toLowerCase();
		if (host === 'localhost' || host === '127.0.0.1' || host === '::1') return true;
		if (
			host.endsWith('.local') ||
			host.endsWith('.lan') ||
			host.endsWith('.home') ||
			host.endsWith('.internal') ||
			host.endsWith('.test') ||
			host.endsWith('.example') ||
			host.endsWith('.invalid')
		)
			return true;
		// Private-network IPv4 literals (office/home LAN, Docker, Tailscale
		// CGNAT 100.64/10, link-local): dev servers there have no R2
		// binding, so their chunk reads 404 by design and need the
		// production fallback exactly like localhost does.
		const octets = host.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
		if (octets) {
			const a = Number(octets[1]);
			const b = Number(octets[2]);
			if (a === 10) return true;
			if (a === 172 && b >= 16 && b <= 31) return true;
			if (a === 192 && b === 168) return true;
			if (a === 100 && b >= 64 && b <= 127) return true;
			if (a === 169 && b === 254) return true;
		}
		return false;
	} catch {
		return false;
	}
}

function readOverride(env?: Record<string, string | undefined>): string | null {
	const raw = (env ?? (import.meta as unknown as { env?: Record<string, string | undefined> }).env)?.[
		'VITE_RNX_REGISTRY'
	];
	if (typeof raw !== 'string') return null;
	const trimmed = raw.trim().replace(/\/$/, '');
	if (!/^https?:\/\//.test(trimmed)) return null;
	return trimmed;
}

// Registry origin for playground stdlib fetches. VITE_RNX_REGISTRY is an
// override with highest priority: when set, it is used exclusively and
// no fallback applies. Without it, the page origin is the registry (so
// production stays same-origin with zero configuration); localhost pages
// additionally get the production fallback, because local dev servers
// have no R2 binding and their chunk reads 404 by design.
export function resolveRegistryOrigin(
	locationOrigin: string,
	env?: Record<string, string | undefined>
): { origin: string; fallback?: string } {
	const override = readOverride(env);
	if (override) return { origin: override };
	if (!locationOrigin.startsWith('http')) {
		throw new Error('stdlib fetch needs an http(s) same-domain registry origin');
	}
	return { origin: locationOrigin, fallback: isDevOrigin(locationOrigin) ? PROD_REGISTRY : undefined };
}

function isChunkEntry(v: unknown): v is ChunkEntry {
	if (typeof v !== 'object' || v === null) return false;
	const o = v as Record<string, unknown>;
	return (
		typeof o['name'] === 'string' &&
		typeof o['size'] === 'number' &&
		typeof o['dir'] === 'boolean' &&
		Array.isArray(o['chunks']) &&
		(o['chunks'] as unknown[]).every((h) => typeof h === 'string')
	);
}

export function stdPackageOf(rest: string): string {
	const top = rest.split('/', 1)[0] ?? '';
	return `@std/${top}`;
}

export function entryCandidates(rest: string): string[] {
	const parts = rest.split('/');
	if (parts.length === 1) return ['src/lib.rnx'];
	const sub = parts.slice(1).join('/');
	return [`src/${sub}.rnx`, `src/${sub}/mod.rnx`, `src/${sub}/index.rnx`];
}

export function findFileEntry(entries: ChunkEntry[], candidates: string[]): ChunkEntry | null {
	for (const want of candidates) {
		const hit = entries.find((e) => !e.dir && e.name === want);
		if (hit) return hit;
	}
	return null;
}

export function concatChunks(parts: Uint8Array[]): Uint8Array {
	let total = 0;
	for (const p of parts) total += p.length;
	const out = new Uint8Array(total);
	let at = 0;
	for (const p of parts) {
		out.set(p, at);
		at += p.length;
	}
	return out;
}

const SHA256_K = new Uint32Array([
	0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
	0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
	0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
	0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
	0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
	0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
	0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
	0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
]);

function rotr32(x: number, n: number): number {
	return (x >>> n) | (x << (32 - n));
}

function hexBytes(bytes: Uint8Array): string {
	return [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
}

// Pure-JS SHA-256 for non-secure contexts (plain http:// on LAN IPs),
// where WebCrypto (`crypto.subtle`) does not exist. Slower than native;
// correctness is what matters — chunk integrity is fail-closed either way.
export function sha256HexFallback(bytes: Uint8Array): string {
	const bitLenLo = (bytes.length << 3) >>> 0;
	const bitLenHi = Math.floor(bytes.length / 0x20000000);
	const paddedLen = (((bytes.length + 8) >> 6) + 1) << 6;
	const msg = new Uint8Array(paddedLen);
	msg.set(bytes);
	msg[bytes.length] = 0x80;
	const dv = new DataView(msg.buffer);
	dv.setUint32(paddedLen - 8, bitLenHi);
	dv.setUint32(paddedLen - 4, bitLenLo);
	let h0 = 0x6a09e667;
	let h1 = 0xbb67ae85;
	let h2 = 0x3c6ef372;
	let h3 = 0xa54ff53a;
	let h4 = 0x510e527f;
	let h5 = 0x9b05688c;
	let h6 = 0x1f83d9ab;
	let h7 = 0x5be0cd19;
	const w = new Uint32Array(64);
	for (let off = 0; off < paddedLen; off += 64) {
		for (let i = 0; i < 16; i++) w[i] = dv.getUint32(off + i * 4);
		for (let i = 16; i < 64; i++) {
			const s0 = rotr32(w[i - 15], 7) ^ rotr32(w[i - 15], 18) ^ (w[i - 15] >>> 3);
			const s1 = rotr32(w[i - 2], 17) ^ rotr32(w[i - 2], 19) ^ (w[i - 2] >>> 10);
			w[i] = (w[i - 16] + s0 + w[i - 7] + s1) | 0;
		}
		let a = h0;
		let b = h1;
		let c = h2;
		let d = h3;
		let e = h4;
		let f = h5;
		let g = h6;
		let h = h7;
		for (let i = 0; i < 64; i++) {
			const s1 = rotr32(e, 6) ^ rotr32(e, 11) ^ rotr32(e, 25);
			const ch = (e & f) ^ (~e & g);
			const t1 = (h + s1 + ch + SHA256_K[i] + w[i]) | 0;
			const s0 = rotr32(a, 2) ^ rotr32(a, 13) ^ rotr32(a, 22);
			const maj = (a & b) ^ (a & c) ^ (b & c);
			const t2 = (s0 + maj) | 0;
			h = g;
			g = f;
			f = e;
			e = (d + t1) | 0;
			d = c;
			c = b;
			b = a;
			a = (t1 + t2) | 0;
		}
		h0 = (h0 + a) | 0;
		h1 = (h1 + b) | 0;
		h2 = (h2 + c) | 0;
		h3 = (h3 + d) | 0;
		h4 = (h4 + e) | 0;
		h5 = (h5 + f) | 0;
		h6 = (h6 + g) | 0;
		h7 = (h7 + h) | 0;
	}
	const out = new Uint8Array(32);
	const odv = new DataView(out.buffer);
	odv.setUint32(0, h0);
	odv.setUint32(4, h1);
	odv.setUint32(8, h2);
	odv.setUint32(12, h3);
	odv.setUint32(16, h4);
	odv.setUint32(20, h5);
	odv.setUint32(24, h6);
	odv.setUint32(28, h7);
	return hexBytes(out);
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
	try {
		if (typeof crypto !== 'undefined' && crypto.subtle) {
			const digest = await crypto.subtle.digest('SHA-256', bytes as BufferSource);
			return hexBytes(new Uint8Array(digest));
		}
	} catch {
		// fall through to the pure-JS implementation below
	}
	return sha256HexFallback(bytes);
}

function stdKey(version: string, rest: string): string {
	return `${STD_KEY_PREFIX}${version}::${rest}`;
}

export function openStdDb(): Promise<IDBDatabase | null> {
	return new Promise((resolve) => {
		try {
			const req = indexedDB.open(STD_DB, 3);
			req.onupgradeneeded = () => {
				const db = req.result;
				if (!db.objectStoreNames.contains('wasm')) db.createObjectStore('wasm');
				if (!db.objectStoreNames.contains(STD_STORE)) db.createObjectStore(STD_STORE);
				if (!db.objectStoreNames.contains('ide')) db.createObjectStore('ide');
			};
			req.onsuccess = () => resolve(req.result);
			req.onerror = () => resolve(null);
		} catch {
			resolve(null);
		}
	});
}

export async function readStdCache(db: IDBDatabase, version: string, rest: string): Promise<string | null> {
	return new Promise((resolve) => {
		try {
			const tx = db.transaction(STD_STORE, 'readonly');
			const req = tx.objectStore(STD_STORE).get(stdKey(version, rest));
			req.onsuccess = () => {
				const v: unknown = req.result;
				if (typeof v === 'object' && v !== null && typeof (v as Record<string, unknown>)['source'] === 'string') {
					resolve((v as { source: string }).source);
				} else {
					resolve(null);
				}
			};
			req.onerror = () => resolve(null);
		} catch {
			resolve(null);
		}
	});
}

async function writeStdCache(db: IDBDatabase, version: string, rest: string, source: string): Promise<void> {
	return new Promise((resolve) => {
		try {
			const tx = db.transaction(STD_STORE, 'readwrite');
			tx.objectStore(STD_STORE).put({ source, fetchedAt: Date.now() }, stdKey(version, rest));
			tx.oncomplete = () => resolve();
			tx.onerror = () => resolve();
		} catch {
			resolve();
		}
	});
}

export async function sweepStaleStd(db: IDBDatabase, version: string): Promise<void> {
	return new Promise((resolve) => {
		try {
			const tx = db.transaction(STD_STORE, 'readwrite');
			const store = tx.objectStore(STD_STORE);
			const req = store.getAllKeys();
			req.onsuccess = () => {
				for (const k of req.result) {
					if (typeof k === 'string' && k.startsWith(STD_KEY_PREFIX) && !k.startsWith(stdKey(version, ''))) {
						store.delete(k);
					}
				}
				resolve();
			};
			req.onerror = () => resolve();
		} catch {
			resolve();
		}
	});
}

export async function fetchStdModule(
	rest: string,
	version: string,
	origin: string,
	fetchImpl: typeof fetch = fetch,
	fallbackOrigin?: string
): Promise<string> {
	try {
		return await fetchStdModuleFrom(rest, version, origin, fetchImpl);
	} catch (e) {
		if (fallbackOrigin && fallbackOrigin !== origin && isHttp404(e)) {
			return await fetchStdModuleFrom(rest, version, fallbackOrigin, fetchImpl);
		}
		throw e;
	}
}

function isHttp404(e: unknown): boolean {
	return e instanceof Error && /HTTP 404/.test(e.message);
}

async function fetchStdModuleFrom(
	rest: string,
	version: string,
	origin: string,
	fetchImpl: typeof fetch
): Promise<string> {
	const full = stdPackageOf(rest);
	const base = `${origin}/api/v0/packages/${full}@${version}`;
	// Tooling reads under a fixed `?origin-direct=1` namespace instead of
	// the bare URL. Shared edge caches can hold manifest generations
	// whose chunks no longer match resolve integrity, and client
	// revalidation headers do not bypass them; the salted key is populated
	// from origin (always live truth, rows are write-once) and the server
	// answers it no-store, so the namespace stays correct. Bypassing costs
	// one origin read per version ever: every caller memoizes by version
	// in IndexedDB.
	const manifestRes = await fetchImpl(`${base}/chunks?origin-direct=1`);
	if (!manifestRes.ok) {
		throw new Error(`stdlib fetch failed: ${full}@${version} chunks manifest answered HTTP ${manifestRes.status}`);
	}
	let entries: ChunkEntry[];
	try {
		const body = (await manifestRes.json()) as { entries: unknown };
		if (!Array.isArray(body.entries) || !body.entries.every(isChunkEntry)) throw new Error('bad shape');
		entries = body.entries as ChunkEntry[];
	} catch {
		throw new Error(`stdlib fetch failed: ${full}@${version} chunks manifest is malformed`);
	}
	const entry = findFileEntry(entries, entryCandidates(rest));
	if (!entry) {
		throw new Error(`stdlib fetch failed: ${full}@${version} ships no ${entryCandidates(rest)[0]}`);
	}
	const parts = await Promise.all(
		entry.chunks.map(async (hash) => {
			if (!/^[0-9a-f]{64}$/.test(hash)) throw new Error(`stdlib fetch failed: bad chunk hash in ${full}@${version}`);
			// Same origin-direct namespace as the manifest fetch above:
			// the salted key is always populated from origin, so chunk
			// bytes match the manifest generation by construction (and are
			// sha-verified below regardless).
			const res = await fetchImpl(`${base}/chunk/${hash}?origin-direct=1`);
			if (!res.ok) throw new Error(`stdlib fetch failed: chunk ${hash.slice(0, 12)}... answered HTTP ${res.status}`);
			const bytes = new Uint8Array(await res.arrayBuffer());
			if ((await sha256Hex(bytes)) !== hash) {
				throw new Error(`stdlib fetch failed: chunk ${hash.slice(0, 12)}... failed its integrity check`);
			}
			return bytes;
		})
	);
	const bytes = concatChunks(parts);
	if (bytes.length > MAX_STD_MODULE_BYTES) {
		throw new Error(`stdlib fetch failed: @std/${rest} exceeds ${MAX_STD_MODULE_BYTES} bytes`);
	}
	return new TextDecoder().decode(bytes);
}

export async function ensureStdModule(
	rest: string,
	version: string,
	origin: string,
	db: IDBDatabase | null,
	fetchImpl: typeof fetch = fetch,
	fallbackOrigin?: string
): Promise<string> {
	if (db) {
		const cached = await readStdCache(db, version, rest);
		if (cached !== null) return cached;
	}
	const source = await fetchStdModule(rest, version, origin, fetchImpl, fallbackOrigin);
	if (db) await writeStdCache(db, version, rest, source);
	return source;
}

export interface StdEngine {
	stdMissing(source: string): Promise<string[]>;
	stdProvide(name: string, source: string): Promise<void>;
	stdPin(): Promise<string>;
}

export async function ensureStdForSource(
	engine: StdEngine,
	source: string,
	version: string,
	origin: string,
	db: IDBDatabase | null,
	fetchImpl: typeof fetch = fetch,
	fallbackOrigin?: string
): Promise<void> {
	for (let round = 0; round < PREFETCH_ROUNDS; round++) {
		const missing = await engine.stdMissing(source);
		if (missing.length === 0) return;
		await Promise.all(
			missing.map(async (name) => {
				const text = await ensureStdModule(name, version, origin, db, fetchImpl, fallbackOrigin);
				await engine.stdProvide(name, text);
			})
		);
	}
	const missing = await engine.stdMissing(source);
	if (missing.length > 0) {
		throw new Error(`stdlib prefetch did not converge; still missing: ${missing.join(', ')}`);
	}
}
