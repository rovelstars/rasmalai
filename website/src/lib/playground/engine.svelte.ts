import type { PlaygroundRunner, RunResult } from '$lib/playground/runner';
import { ensureStdForSource, ensureStdModule, openStdDb, sweepStaleStd } from '$lib/playground/stdvfs';

export type EngineState = 'unloaded' | 'downloading' | 'ready' | 'running' | 'complete' | 'error';

export interface CompletionEntry {
	label: string;
	kind: number;
	detail: string;
	insertText: string;
}

export interface HoverInfo {
	signature: string;
	docs: string;
	start: number;
	end: number;
}

export interface ProjectDiag {
	file: string;
	line: number;
	col: number;
	start: number;
	end: number;
	code: string;
	severity: string;
	message: string;
}

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
			const req = indexedDB.open(IDB_NAME, 3);
			req.onupgradeneeded = () => {
				if (!req.result.objectStoreNames.contains(IDB_STORE)) req.result.createObjectStore(IDB_STORE);
				if (!req.result.objectStoreNames.contains('std')) req.result.createObjectStore('std');
				if (!req.result.objectStoreNames.contains('ide')) req.result.createObjectStore('ide');
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
					if (k !== keep && !(typeof k === 'string' && k.startsWith('std::'))) store.delete(k);
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
	private stdPin: string | null = null;
	private stdDb: IDBDatabase | null = null;

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
			await this.preloadStdlib();
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
		await this.ensureStd(source);
		return this.runner.check(source);
	}

	async tokens(source: string): Promise<string> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		return this.runner.tokens(source);
	}

	async formatSource(source: string): Promise<string> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		const raw = await this.runner.format(source);
		const parsed = JSON.parse(raw) as { ok: boolean; output?: string; error?: string };
		if (!parsed.ok || typeof parsed.output !== 'string') {
			throw new Error(parsed.error ?? 'format failed');
		}
		return parsed.output;
	}

	async complete(source: string, line: number, character: number): Promise<CompletionEntry[]> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		const raw = await this.runner.complete(source, line, character);
		const parsed = JSON.parse(raw) as { items?: CompletionEntry[]; error?: string };
		if (parsed.error) throw new Error(parsed.error);
		return parsed.items ?? [];
	}

	async hover(source: string, line: number, character: number): Promise<HoverInfo | null> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		const raw = await this.runner.hover(source, line, character);
		const parsed = JSON.parse(raw) as { empty?: boolean; error?: string } & Partial<HoverInfo>;
		if (parsed.error) throw new Error(parsed.error);
		if (parsed.empty || typeof parsed.signature !== 'string') return null;
		return {
			signature: parsed.signature,
			docs: typeof parsed.docs === 'string' ? parsed.docs : '',
			start: typeof parsed.start === 'number' ? parsed.start : 0,
			end: typeof parsed.end === 'number' ? parsed.end : 0
		};
	}

	async diagJson(source: string): Promise<ProjectDiag[]> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		const raw = await this.runner.diagJson(source);
		return JSON.parse(raw) as ProjectDiag[];
	}

	async diagProject(filesJson: string, entry: string): Promise<ProjectDiag[]> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		const files = JSON.parse(filesJson) as Record<string, string>;
		await this.ensureStdMany(Object.values(files));
		const raw = await this.runner.diagProject(filesJson, entry);
		return JSON.parse(raw) as ProjectDiag[];
	}

	async checkProject(filesJson: string, entry: string): Promise<string> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		const files = JSON.parse(filesJson) as Record<string, string>;
		await this.ensureStdMany(Object.values(files));
		return this.runner.checkProject(filesJson, entry);
	}

	async runProject(filesJson: string, entry: string): Promise<RunResult> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		this.state = 'running';
		this.detail = '';
		try {
			const files = JSON.parse(filesJson) as Record<string, string>;
			await this.ensureStdMany(Object.values(files));
			const res = await this.runner.runProject(filesJson, entry);
			this.state = 'complete';
			return res;
		} catch (e) {
			this.state = 'error';
			this.detail = e instanceof Error ? e.message : 'Run failed.';
			throw e;
		}
	}

	async testProject(filesJson: string, entry: string): Promise<RunResult> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		this.state = 'running';
		this.detail = '';
		try {
			const files = JSON.parse(filesJson) as Record<string, string>;
			await this.ensureStdMany(Object.values(files));
			const res = await this.runner.testProject(filesJson, entry);
			this.state = 'complete';
			return res;
		} catch (e) {
			this.state = 'error';
			this.detail = e instanceof Error ? e.message : 'Run failed.';
			throw e;
		}
	}

	async run(source: string): Promise<RunResult> {
		await this.ensureLoaded();
		if (!this.runner) throw new Error('engine unavailable');
		this.state = 'running';
		this.detail = '';
		try {
			await this.ensureStd(source);
			const res = await this.runner.run(source);
			this.state = 'complete';
			return res;
		} catch (e) {
			this.state = 'error';
			this.detail = e instanceof Error ? e.message : 'Run failed.';
			throw e;
		}
	}

	private registryOrigin(): string {
		try {
			if (typeof location !== 'undefined' && location.origin.startsWith('http')) return location.origin;
		} catch {
			// fall through to the error below
		}
		throw new Error('stdlib fetch needs an http(s) same-domain registry origin');
	}

	private devFallbackOrigin(origin: string): string | undefined {
		try {
			const host = new URL(origin).hostname;
			if (host === 'localhost' || host === '127.0.0.1' || host.endsWith('.local')) {
				return 'https://rasmalai.rovelstars.com';
			}
		} catch {
			// unparseable origin: no fallback
		}
		return undefined;
	}

	private async preloadStdlib(): Promise<void> {
		if (!this.runner) return;
		this.stdDb = await openStdDb();
		this.stdPin = await this.runner.stdPin();
		const origin = this.registryOrigin();
		const prelude = await ensureStdModule('prelude', this.stdPin, origin, this.stdDb, fetch, this.devFallbackOrigin(origin));
		await this.runner.stdProvide('prelude', prelude);
		if (this.stdDb && this.stdPin) await sweepStaleStd(this.stdDb, this.stdPin);
	}

	private async ensureStd(source: string): Promise<void> {
		if (!this.runner || !this.stdPin) return;
		try {
			const origin = this.registryOrigin();
			await ensureStdForSource(this.runner, source, this.stdPin, origin, this.stdDb, fetch, this.devFallbackOrigin(origin));
		} catch (e) {
			if (e instanceof Error && /timed out|did not converge/.test(e.message)) throw e;
		}
	}

	private async ensureStdMany(sources: string[]): Promise<void> {
		for (const source of sources) {
			await this.ensureStd(source);
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
