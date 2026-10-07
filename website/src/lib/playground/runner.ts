export interface RunResult {
	output: string;
	duration: number;
	timedOut: boolean;
}

const TIMEOUT_MS = 3000;

export class PlaygroundRunner {
	private worker: Worker | null = null;
	private seq = 0;
	private initReady: Promise<string> | null = null;
	private wasmUrl: string | undefined;
	private pending = new Map<
		number,
		{
			resolve: (v: string) => void;
			reject: (e: Error) => void;
			timer: ReturnType<typeof setTimeout>;
			type: string;
		}
	>();

	constructor(wasmUrl?: string) {
		this.wasmUrl = wasmUrl;
		this.spawn(wasmUrl);
	}

	async ping(): Promise<void> {
		await this.initReady;
	}

	private spawn(wasmUrl?: string): Worker {
		if (this.worker) this.worker.terminate();
		this.pending.forEach((p) => {
			clearTimeout(p.timer);
			p.reject(new Error('worker restarted'));
		});
		this.pending.clear();
		const w = new Worker(new URL('../workers/playground.worker.ts', import.meta.url), {
			type: 'module'
		});
		w.onmessage = (e: MessageEvent) => {
			const msg = e.data as { type: string; id: number; json?: string; text?: string; output?: string; duration?: number; totalMs?: number; iters?: number; missing?: string[]; version?: string };
			const p = this.pending.get(msg.id);
			if (!p) return;
			this.pending.delete(msg.id);
			clearTimeout(p.timer);
			if (msg.type === 'INIT_DONE') p.resolve('ok');
			else if (msg.type === 'STD_MISSING' && msg.missing !== undefined) p.resolve(JSON.stringify(msg.missing));
			else if (msg.type === 'STD_PROVIDED') p.resolve('ok');
			else if (msg.type === 'STD_PIN' && msg.version !== undefined) p.resolve(msg.version);
			else if (msg.type === 'DIAG' && msg.text !== undefined && (p.type === 'RUN' || p.type.startsWith('STD_'))) p.reject(new Error(msg.text));
			else if (msg.type === 'TOKENS' && msg.json !== undefined) p.resolve(msg.json);
			else if (msg.type === 'JSON' && msg.json !== undefined) p.resolve(msg.json);
			else if (msg.type === 'DIAG' && msg.text !== undefined) p.resolve(msg.text);
			else if (msg.type === 'RESULT') {
				p.resolve(JSON.stringify({ output: msg.output ?? '', duration: msg.duration ?? 0 }));
			} else if (msg.type === 'BENCH_RESULT') {
				p.resolve(JSON.stringify({ totalMs: msg.totalMs ?? 0, iters: msg.iters ?? 0 }));
			} else {
				p.reject(new Error('unexpected worker message'));
			}
		};
		w.onerror = () => {
			this.restart();
		};
		this.worker = w;
		const id = ++this.seq;
		this.initReady = new Promise<string>((resolve, reject) => {
			const timer = setTimeout(() => {
				this.pending.delete(id);
				reject(new Error('engine init timed out'));
			}, 30000);
			this.pending.set(id, { resolve, reject, timer, type: 'INIT' });
		});
		w.postMessage({ type: 'INIT', id, wasmUrl });
		return w;
	}

	private async call(type: 'TOKENS' | 'CHECK' | 'RUN' | 'BENCH', source: string, timeoutMs: number, iters = 0): Promise<string> {
		const w = this.worker ?? this.spawn(this.wasmUrl);
		await this.initReady;
		const id = ++this.seq;
		return new Promise<string>((resolve, reject) => {
			const timer = setTimeout(() => {
				this.pending.delete(id);
				if (type === 'RUN') {
					this.spawn(this.wasmUrl);
					reject(new Error(`run exceeded ${timeoutMs} ms; worker restarted`));
				} else {
					reject(new Error(`${type} timed out`));
				}
			}, timeoutMs);
			this.pending.set(id, { resolve, reject, timer, type });
			w.postMessage({ type, id, source, iters });
		});
	}

	check(source: string): Promise<string> {
		return this.call('CHECK', source, TIMEOUT_MS);
	}

	private async post(type: string, payload: Record<string, unknown>, timeoutMs: number): Promise<string> {
		const w = this.worker ?? this.spawn(this.wasmUrl);
		await this.initReady;
		const id = ++this.seq;
		return new Promise<string>((resolve, reject) => {
			const timer = setTimeout(() => {
				this.pending.delete(id);
				if (type === 'RUN_PROJECT' || type === 'TEST_PROJECT') {
					this.spawn(this.wasmUrl);
					reject(new Error(`run exceeded ${timeoutMs} ms; worker restarted`));
				} else {
					reject(new Error(`${type} timed out`));
				}
			}, timeoutMs);
			this.pending.set(id, { resolve, reject, timer, type });
			w.postMessage({ type, id, ...payload });
		});
	}

	format(source: string): Promise<string> {
		return this.post('FORMAT', { source }, TIMEOUT_MS);
	}

	complete(source: string, line: number, character: number): Promise<string> {
		return this.post('COMPLETE', { source, line, character }, TIMEOUT_MS);
	}

	hover(source: string, line: number, character: number): Promise<string> {
		return this.post('HOVER', { source, line, character }, TIMEOUT_MS);
	}

	diagJson(source: string): Promise<string> {
		return this.post('DIAG_JSON', { source }, TIMEOUT_MS);
	}

	checkProject(filesJson: string, entry: string): Promise<string> {
		return this.post('CHECK_PROJECT', { source: filesJson, entry }, TIMEOUT_MS);
	}

	diagProject(filesJson: string, entry: string): Promise<string> {
		return this.post('DIAG_PROJECT', { source: filesJson, entry }, TIMEOUT_MS);
	}

	async runProject(filesJson: string, entry: string): Promise<RunResult> {
		try {
			const raw = await this.post('RUN_PROJECT', { source: filesJson, entry }, TIMEOUT_MS);
			const parsed = JSON.parse(raw) as { output: string; duration: number };
			return { output: parsed.output, duration: parsed.duration, timedOut: false };
		} catch (e) {
			if (e instanceof Error && e.message.includes('worker restarted')) {
				return { output: '', duration: TIMEOUT_MS, timedOut: true };
			}
			throw e;
		}
	}

	async testProject(filesJson: string, entry: string): Promise<RunResult> {
		try {
			const raw = await this.post('TEST_PROJECT', { source: filesJson, entry }, TIMEOUT_MS);
			const parsed = JSON.parse(raw) as { output: string; duration: number };
			return { output: parsed.output, duration: parsed.duration, timedOut: false };
		} catch (e) {
			if (e instanceof Error && e.message.includes('worker restarted')) {
				return { output: '', duration: TIMEOUT_MS, timedOut: true };
			}
			throw e;
		}
	}

	async stdMissing(source: string): Promise<string[]> {
		const w = this.worker ?? this.spawn(this.wasmUrl);
		await this.initReady;
		const id = ++this.seq;
		return new Promise<string[]>((resolve, reject) => {
			const timer = setTimeout(() => {
				this.pending.delete(id);
				reject(new Error('STD_MISSING timed out'));
			}, TIMEOUT_MS);
			this.pending.set(id, { resolve: (v) => resolve(JSON.parse(v) as string[]), reject, timer, type: 'STD_MISSING' });
			w.postMessage({ type: 'STD_MISSING', id, source });
		});
	}

	async stdProvide(name: string, source: string): Promise<void> {
		const w = this.worker ?? this.spawn(this.wasmUrl);
		await this.initReady;
		const id = ++this.seq;
		return new Promise<void>((resolve, reject) => {
			const timer = setTimeout(() => {
				this.pending.delete(id);
				reject(new Error('STD_PROVIDE timed out'));
			}, TIMEOUT_MS);
			this.pending.set(id, { resolve: () => resolve(), reject, timer, type: 'STD_PROVIDE' });
			w.postMessage({ type: 'STD_PROVIDE', id, name, source });
		});
	}

	async stdPin(): Promise<string> {
		const w = this.worker ?? this.spawn(this.wasmUrl);
		await this.initReady;
		const id = ++this.seq;
		return new Promise<string>((resolve, reject) => {
			const timer = setTimeout(() => {
				this.pending.delete(id);
				reject(new Error('STD_PIN timed out'));
			}, TIMEOUT_MS);
			this.pending.set(id, { resolve, reject, timer, type: 'STD_PIN' });
			w.postMessage({ type: 'STD_PIN', id });
		});
	}

	async bench(source: string, iters: number): Promise<{ totalMs: number; iters: number }> {
		const raw = await this.call('BENCH', source, 30000, iters);
		return JSON.parse(raw) as { totalMs: number; iters: number };
	}

	tokens(source: string): Promise<string> {
		return this.call('TOKENS', source, TIMEOUT_MS);
	}

	async run(source: string): Promise<RunResult> {
		try {
			const raw = await this.call('RUN', source, TIMEOUT_MS);
			const parsed = JSON.parse(raw) as { output: string; duration: number };
			return { output: parsed.output, duration: parsed.duration, timedOut: false };
		} catch (e) {
			if (e instanceof Error && e.message.includes('worker restarted')) {
				return { output: '', duration: TIMEOUT_MS, timedOut: true };
			}
			throw e;
		}
	}

	restart() {
		this.spawn(this.wasmUrl);
	}

	dispose() {
		this.pending.forEach((p) => {
			clearTimeout(p.timer);
			p.reject(new Error('disposed'));
		});
		this.pending.clear();
		this.worker?.terminate();
		this.worker = null;
	}
}
