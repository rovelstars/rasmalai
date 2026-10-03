import init, { tokenize, check, run } from '$lib/wasm/wasm_playground.js';
import defaultWasmUrl from '$lib/wasm/wasm_playground_bg.wasm?url';

export type WorkerOut =
	| { type: 'READY' }
	| { type: 'INIT_DONE'; id: number }
	| { type: 'TOKENS'; id: number; json: string }
	| { type: 'DIAG'; id: number; text?: string }
	| { type: 'RESULT'; id: number; output: string; duration: number }
	| { type: 'BENCH_RESULT'; id: number; totalMs: number; iters?: number };

// Lazily initialized: the 1.1MB binary is fetched only after the main
// thread posts INIT with a (possibly cached) object URL. CHECK/RUN
// arriving first fall back to the bundled asset URL.
let ready: Promise<unknown> | null = null;

function ensureInit(url?: string): Promise<unknown> {
	if (!ready) {
		ready = init(url ?? defaultWasmUrl).catch((e) => {
			postMessage({ type: 'DIAG', id: -1, text: `worker init failed: ${String(e)}` });
			throw e;
		});
	}
	return ready;
}

self.onmessage = async (e: MessageEvent) => {
	const msg = e.data as { type: string; id: number; source: string; iters?: number; wasmUrl?: string };
	try {
		if (msg.type === 'INIT') {
			await ensureInit(msg.wasmUrl);
			postMessage({ type: 'INIT_DONE', id: msg.id } as WorkerOut);
			return;
		}
		await ensureInit();
		if (msg.type === 'TOKENS') {
			postMessage({ type: 'TOKENS', id: msg.id, json: tokenize(msg.source) } as WorkerOut);
		} else if (msg.type === 'CHECK') {
			postMessage({ type: 'DIAG', id: msg.id, text: check(msg.source) } as WorkerOut);
		} else if (msg.type === 'BENCH') {
			const iters = Math.min(msg.iters ?? 200, 5000);
			const t0 = performance.now();
			for (let i = 0; i < iters; i++) check(msg.source);
			postMessage({ type: 'BENCH_RESULT', id: msg.id, totalMs: performance.now() - t0, iters } as WorkerOut);
		} else if (msg.type === 'RUN') {
			const t0 = performance.now();
			const output = run(msg.source);
			postMessage({
				type: 'RESULT',
				id: msg.id,
				output,
				duration: performance.now() - t0
			} as WorkerOut);
		}
	} catch (err) {
		postMessage({ type: 'DIAG', id: msg.id, text: `worker error: ${String(err)}` } as WorkerOut);
	}
};
