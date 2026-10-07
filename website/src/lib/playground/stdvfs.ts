// Registry-backed stdlib file view for the playground engine.
//
// The wasm engine ships no embedded `@std/*` sources. The main thread
// preloads `@std/prelude` at engine download and lazily fetches every
// other imported module from the same-domain registry (fixed 15_REGISTRY
// chunk paths, never user-supplied URLs), verifies each chunk against its
// sha256, and hands the bytes to the engine over the worker protocol.
// Versioned package URLs are immutable, so fetched modules cache in
// IndexedDB beside the engine bytes forever.

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

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
	const digest = await crypto.subtle.digest('SHA-256', bytes as BufferSource);
	return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
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
	const base = `${origin}/api/packages/${full}@${version}`;
	// See the chunk-fetch note below: manifests are versioned and
	// immutable, so bypassing shared edge caches costs one origin read
	// per version ever (the parsed result is not cached here, but every
	// caller memoizes by version in IndexedDB).
	const manifestRes = await fetchImpl(`${base}/chunks`, { cache: 'reload' });
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
			// Bypass shared edge caches: some nodes hold pre-CORS generations
			// of these URLs with year-long TTLs. Reads stay correct either
			// way (every chunk is sha-verified below), but only a fresh
			// origin response carries the CORS headers browsers need.
			// Volume is negligible: each module downloads once per version
			// and then serves from IndexedDB forever.
			const res = await fetchImpl(`${base}/chunk/${hash}`, { cache: 'reload' });
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
