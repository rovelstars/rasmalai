import type { PlaygroundRunner, RunResult } from '$lib/playground/runner';

export type EngineState = 'unloaded' | 'downloading' | 'ready' | 'running' | 'complete' | 'error';

const IDB_NAME = 'rnx-engine';
const IDB_STORE = 'wasm';

interface CachedEngine {
	bytes: ArrayBuffer;
	len: number;
	etag: string | null;
}

function isCachedEngine(v: unknown): v is CachedEngine {
	if (typeof v !== 'object' || v === null) return false;
	const o = v as Record<string, unknown>;
	return o['bytes'] instanceof ArrayBuffer && typeof o['len'] === 'number';
}

function openDb(): Promise<IDBDatabase | null> {
	return new Promise((resolve) => {
		try {
			const req = indexedDB.open(IDB_NAME, 1);
			req.onupgradeneeded = () => {
				req.result.createObjectStore(IDB_STORE);
			};
			req.onsuccess = () => resolve(req.result);
			req.onerror = () => resolve(null);
		} catch {
			resolve(null);
		}
	});
}

async function readCache(db: IDBDatabase, key: string): Promise<CachedEngine | null> {
	return new Promise((resolve) => {
		try {
			const tx = db.transaction(IDB_STORE, 'readonly');
			const req = tx.objectStore(IDB_STORE).get(key);
			req.onsuccess = () => {
				const v: unknown = req.result;
				resolve(isCachedEngine(v) ? v : null);
			};
			req.onerror = () => resolve(null);
		} catch {
			resolve(null);
		}
	});
}

async function writeCache(db: IDBDatabase, key: string, entry: CachedEngine): Promise<void> {
	return new Promise((resolve) => {
		try {
			const tx = db.transaction(IDB_STORE, 'readwrite');
			tx.objectStore(IDB_STORE).put(entry, key);
			tx.oncomplete = () => resolve();
			tx.onerror = () => resolve();
		} catch {
			resolve();
		}
	});
}

async function sweepCache(db: IDBDatabase, keep: string): Promise<void> {
	return new Promise((resolve) => {
		try {
			const tx = db.transaction(IDB_STORE, 'readwrite');
			const store = tx.objectStore(IDB_STORE);
			const req = store.getAllKeys();
			req.onsuccess = () => {
				for (const k of req.result) {
					if (k !== keep) store.delete(k);
				}
				resolve();
			};
			req.onerror = () => resolve();
		} catch {
			resolve();
		}
	});
}

async function headMeta(
	assetUrl: string
): Promise<{ len: number; etag: string | null } | null> {
	try {
		const res = await fetch(assetUrl, { method: 'HEAD' });
		if (!res.ok) return null;
		return {
			len: Number(res.headers.get('content-length') ?? 0),
			etag: res.headers.get('etag')
		};
	} catch {
		return null;
	}
}

async function fetchBytes(
	assetUrl: string,
	onProgress: (fraction: number | null) => void
): Promise<ArrayBuffer> {
	const res = await fetch(assetUrl);
	if (!res.ok || !res.body) throw new Error(`engine download failed: HTTP ${res.status}`);
	const total = Number(res.headers.get('content-length') ?? 0);
	const reader = res.body.getReader();
	const chunks: Uint8Array[] = [];
	let received = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		chunks.push(value);
		received += value.length;
		onProgress(total > 0 ? received / total : null);
	}
	const out = new Uint8Array(received);
	let at = 0;
	for (const c of chunks) {
		out.set(c, at);
		at += c.length;
	}
	return out.buffer as ArrayBuffer;
}

class Engine {
	state = $state<EngineState>('unloaded');
	progress = $state<number | null>(null);
	detail = $state('');
	private runner: PlaygroundRunner | null = null;
	private loading: Promise<void> | null = null;
	private RunnerClass: typeof PlaygroundRunner | null = null;

	get loaded(): boolean {
		return this.runner !== null;
	}

	async ensureLoaded(onProgress?: (fraction: number | null, fromCache?: boolean) => void): Promise<void> {
		if (this.runner) {
			this.state = 'ready';
			return;
		}
		if (this.loading) return this.loading;
		this.loading = this.load(onProgress);
		try {
			await this.loading;
		} finally {
			this.loading = null;
		}
	}

	private async load(onProgress?: (fraction: number | null, fromCache?: boolean) => void): Promise<void> {
		this.state = 'downloading';
		this.progress = 0;
		try {
			const report = (f: number | null, fromCache = false) => {
				this.progress = f;
				this.detail = fromCache
					? 'Engine restored from local cache.'
					: f === null
						? 'Downloading compiler engine...'
						: `Downloading compiler engine... ${Math.round(f * 100)}%`;
				onProgress?.(f, fromCache);
			};
			const [{ PlaygroundRunner: Ctor }, { default: wasmAssetUrl }] = await Promise.all([
				import('./runner'),
				import('$lib/wasm/wasm_playground_bg.wasm?url')
			]);
			this.RunnerClass = Ctor;
			const cacheKey = `wasm::${wasmAssetUrl as string}`;
			let bytes: ArrayBuffer | null = null;
			const db = await openDb();
			if (db) {
				const [cached, meta] = await Promise.all([
					readCache(db, cacheKey),
					headMeta(wasmAssetUrl as string)
				]);
				const fresh =
					cached !== null &&
					meta !== null &&
					meta.len > 0 &&
					cached.len === meta.len &&
					(meta.etag === null || cached.etag === null || cached.etag === meta.etag);
				if (fresh && cached) {
					bytes = cached.bytes;
					report(1, true);
				}
				await sweepCache(db, cacheKey);
			}
			if (!bytes) {
				report(0);
				bytes = await fetchBytes(wasmAssetUrl as string, report);
				if (db) {
					const again = await headMeta(wasmAssetUrl as string);
					await writeCache(db, cacheKey, {
						bytes,
						len: bytes.byteLength,
						etag: again?.etag ?? null
					});
				}
			}
			const blobUrl = URL.createObjectURL(new Blob([bytes], { type: 'application/wasm' }));
			const runner = new Ctor(blobUrl);
			await runner.ping();
			this.runner = runner;
			this.state = 'ready';
			this.progress = 1;
			this.detail = '';
		} catch (e) {
			this.state = 'error';
			this.detail = e instanceof Error ? e.message : 'Engine failed to load.';
			this.progress = null;
			throw e;
		}
	}

	async check(source: string): Promise<string> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		return this.runner.check(source);
	}

	async tokens(source: string): Promise<string> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		return this.runner.tokens(source);
	}

	async run(source: string): Promise<RunResult> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		this.state = 'running';
		this.detail = '';
		try {
			const res = await this.runner.run(source);
			this.state = 'complete';
			return res;
		} catch (e) {
			this.state = 'error';
			this.detail = e instanceof Error ? e.message : 'Run failed.';
			throw e;
		}
	}

	markRunning() {
		if (this.runner) this.state = 'running';
	}

	markComplete() {
		if (this.runner) this.state = 'complete';
	}
}

export const engine = new Engine();
