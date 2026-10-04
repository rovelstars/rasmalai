export interface BenchmarkResult {
	lang: string;
	mode: string;
	label: string;
	runtime_ms: number;
	peak_rss_mb: number;
	build_ms: number;
	build: string;
	run: string;
	artifact: boolean;
}

export interface BenchmarkSnapshot {
	benchmarks: Record<string, unknown>;
	system?: { cpu?: string; date?: string };
	versions?: Record<string, string>;
}

const isNum = (v: unknown): v is number =>
	typeof v === 'number' && Number.isFinite(v) && v >= 0;

export function unwrapEnvelope(data: unknown): BenchmarkSnapshot | null {
	if (!data || typeof data !== 'object') return null;
	const obj = data as Record<string, unknown>;
	const inner = obj['snapshot'];
	if (inner && typeof inner === 'object') return inner as BenchmarkSnapshot;
	return data as BenchmarkSnapshot;
}

export function validateSnapshot(data: unknown): string | null {
	const snap = unwrapEnvelope(data);
	if (!snap || typeof snap !== 'object') return 'not an object';
	const benches = (snap as BenchmarkSnapshot).benchmarks;
	if (!benches || typeof benches !== 'object') return 'missing benchmarks';
	const ids = Object.keys(benches);
	if (ids.length === 0) return 'no benchmarks';
	for (const id of ids) {
		const b = (benches as Record<string, unknown>)[id] as Record<string, unknown>;
		if (!b || typeof b !== 'object') return `${id}: not an object`;
		if (
			typeof b['name'] !== 'string' ||
			typeof b['category'] !== 'string' ||
			typeof b['description'] !== 'string' ||
			!Array.isArray(b['results']) ||
			(b['results'] as unknown[]).length === 0
		) {
			return `${id}: missing name/category/description/results`;
		}
		const labels = new Set<string>();
		for (const r of b['results'] as BenchmarkResult[]) {
			const where = `${id}/${r.label ?? r.lang}`;
			if (typeof r.lang !== 'string' || typeof r.mode !== 'string') {
				return `${where}: missing lang/mode`;
			}
			if (r.mode !== 'dev' && r.mode !== 'rel') return `${where}: mode must be dev or rel`;
			if (r.label !== `${r.lang} ${r.mode}`) return `${where}: bad label`;
			if (labels.has(r.label)) return `${where}: duplicate label`;
			labels.add(r.label);
			if (!isNum(r.runtime_ms) || !isNum(r.peak_rss_mb) || !isNum(r.build_ms)) {
				return `${where}: invalid numbers`;
			}
			if (typeof r.build !== 'string' || !r.build) return `${where}: missing build`;
			if (typeof r.run !== 'string' || !r.run) return `${where}: missing run`;
			if (typeof r.artifact !== 'boolean') return `${where}: missing artifact flag`;
			if (r.artifact && r.build_ms === 0) return `${where}: artifact with zero build_ms`;
		}
	}
	return null;
}
