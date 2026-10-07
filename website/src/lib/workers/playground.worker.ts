import init, {
	tokenize,
	check,
	run,
	std_missing,
	std_provide,
	std_pin_version,
	format_source,
	complete,
	hover,
	diagnostics_json,
	check_project,
	run_project,
	test_project,
	diagnostics_project
} from '$lib/wasm/wasm_playground.js';
import defaultWasmUrl from '$lib/wasm/wasm_playground_bg.wasm?url';

export type WorkerOut =
	| { type: 'READY' }
	| { type: 'INIT_DONE'; id: number }
	| { type: 'TOKENS'; id: number; json: string }
	| { type: 'JSON'; id: number; json: string }
	| { type: 'DIAG'; id: number; text?: string }
	| { type: 'RESULT'; id: number; output: string; duration: number }
	| { type: 'BENCH_RESULT'; id: number; totalMs: number; iters?: number }
	| { type: 'STD_MISSING'; id: number; missing: string[] }
	| { type: 'STD_PROVIDED'; id: number }
	| { type: 'STD_PIN'; id: number; version: string };

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

function jsonOut(id: number, json: string): void {
	postMessage({ type: 'JSON', id, json } as WorkerOut);
}

self.onmessage = async (e: MessageEvent) => {
	const msg = e.data as {
		type: string;
		id: number;
		source: string;
		name?: string;
		entry?: string;
		line?: number;
		character?: number;
		iters?: number;
		wasmUrl?: string;
	};
	try {
		if (msg.type === 'INIT') {
			await ensureInit(msg.wasmUrl);
			postMessage({ type: 'INIT_DONE', id: msg.id } as WorkerOut);
			return;
		}
		await ensureInit();
		if (msg.type === 'STD_MISSING') {
			const raw = std_missing(msg.source) as string;
			const parsed = JSON.parse(raw) as { missing?: string[]; error?: string };
			if (parsed.error) throw new Error(parsed.error);
			postMessage({ type: 'STD_MISSING', id: msg.id, missing: parsed.missing ?? [] } as WorkerOut);
		} else if (msg.type === 'STD_PROVIDE') {
			const raw = std_provide(msg.name ?? '', msg.source) as string;
			const parsed = JSON.parse(raw) as { ok: boolean; error?: string };
			if (!parsed.ok) throw new Error(parsed.error ?? 'std_provide failed');
			postMessage({ type: 'STD_PROVIDED', id: msg.id } as WorkerOut);
		} else if (msg.type === 'STD_PIN') {
			postMessage({ type: 'STD_PIN', id: msg.id, version: std_pin_version() } as WorkerOut);
		} else if (msg.type === 'TOKENS') {
			postMessage({ type: 'TOKENS', id: msg.id, json: tokenize(msg.source) } as WorkerOut);
		} else if (msg.type === 'CHECK') {
			postMessage({ type: 'DIAG', id: msg.id, text: check(msg.source) } as WorkerOut);
		} else if (msg.type === 'FORMAT') {
			jsonOut(msg.id, format_source(msg.source) as string);
		} else if (msg.type === 'COMPLETE') {
			jsonOut(msg.id, complete(msg.source, msg.line ?? 0, msg.character ?? 0) as string);
		} else if (msg.type === 'HOVER') {
			jsonOut(msg.id, hover(msg.source, msg.line ?? 0, msg.character ?? 0) as string);
		} else if (msg.type === 'DIAG_JSON') {
			jsonOut(msg.id, diagnostics_json(msg.source) as string);
		} else if (msg.type === 'CHECK_PROJECT') {
			postMessage({ type: 'DIAG', id: msg.id, text: check_project(msg.source, msg.entry ?? 'main.rnx') } as WorkerOut);
		} else if (msg.type === 'DIAG_PROJECT') {
			jsonOut(msg.id, diagnostics_project(msg.source, msg.entry ?? 'main.rnx') as string);
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
		} else if (msg.type === 'RUN_PROJECT' || msg.type === 'TEST_PROJECT') {
			const t0 = performance.now();
			const output =
				msg.type === 'RUN_PROJECT'
					? (run_project(msg.source, msg.entry ?? 'main.rnx') as string)
					: (test_project(msg.source, msg.entry ?? 'main.rnx') as string);
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
