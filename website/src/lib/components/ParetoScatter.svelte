<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { LoaderCircle } from 'lucide-svelte';
	import CountUp from '$lib/motion/CountUp.svelte';
	import { reveal } from '$lib/motion/reveal';

	interface Result {
		lang: string;
		mode: 'dev' | 'rel';
		label: string;
		runtime_ms: number;
		peak_rss_mb: number;
		build_ms: number;
		build: string;
		run: string;
		artifact: boolean;
	}
	interface Bench {
		name: string;
		category: string;
		description: string;
		results: Result[];
	}
	interface Snapshot {
		benchmarks: Record<string, Bench>;
		system?: { cpu?: string; date?: string };
		versions?: Record<string, string>;
	}

	// The monthly benchmark workflow publishes snapshots to Turso via
	// POST /api/benchmarks. The chart reads the live API at runtime and
	// renders the empty state below when no snapshot exists yet.
	export const benchmarksUrl = '/api/benchmarks';

	async function loadSnapshot(): Promise<Snapshot | null> {
		const res = await fetch(benchmarksUrl).catch(() => null);
		const data = res?.ok ? await res.json().catch(() => null) : null;
		const snap = data?.snapshot ?? data;
		if (snap?.benchmarks && typeof snap.benchmarks === 'object') return snap as Snapshot;
		return null;
	}

	let snapshot = $state<Snapshot | null>(null);
	let noData = $state(false);

	onMount(async () => {
		snapshot = await loadSnapshot();
		if (!snapshot) noData = true;
	});

	const BENCHES: { id: string; bench: Bench }[] = $derived(
		Object.entries(snapshot?.benchmarks ?? {}).map(([id, bench]) => ({ id, bench }))
	);
	const LANG_ORDER = ['rnx', 'c', 'rust', 'go', 'node', 'java', 'dart'];
	const LANG_COLOR: Record<string, string> = {
		rnx: '#61ffca',
		c: '#8b9bb4',
		rust: '#ff9e64',
		go: '#82e2ff',
		node: '#7ee787',
		java: '#ff6767',
		dart: '#79c0ff'
	};

	const measuredOn = $derived(snapshot?.system?.cpu ?? 'unknown machine');
	const measuredAt = $derived((snapshot?.system?.date ?? '').slice(0, 10));
	const versions = $derived(snapshot?.versions ?? {});

	let selId = $state('');
	let yMode = $state<'ram' | 'build'>('ram');
	let mode = $state<'all' | 'dev' | 'rel'>('all');
	let hovered = $state<string | null>(null);

	const sel = $derived(BENCHES.find((b) => b.id === selId)?.bench ?? BENCHES[0]?.bench);

	$effect(() => {
		if (!selId && BENCHES[0]) selId = BENCHES[0].id;
	});
	// Every point is a (language, mode) pair. Sorting by language then mode
	// keeps a language's dev and rel dots adjacent so the gap between them
	// reads as that language's own dev/rel tradeoff.
	const ordered = $derived(
		[...(sel?.results ?? [])]
			.filter((r) => mode === 'all' || r.mode === mode)
			.sort(
				(a, b) =>
					LANG_ORDER.indexOf(a.lang) - LANG_ORDER.indexOf(b.lang) ||
					(a.mode === 'dev' ? -1 : 1) - (b.mode === 'dev' ? -1 : 1)
			)
	);
	const yOf = $derived((r: Result) => (yMode === 'ram' ? r.peak_rss_mb : r.build_ms));
	const yLabel = $derived(
		yMode === 'ram' ? 'peak RAM (MB, log scale)' : 'build time (ms, log scale)'
	);
	const yMetric = $derived(yMode === 'ram' ? 'peak memory' : 'build time');
	// The headline number and the speedup denominator. In release mode that is
	// rnx release; in dev mode, where rnx release is filtered out, it is rnx dev.
	const rnx = $derived(
		ordered.find((r) => r.lang === 'rnx' && r.mode === 'rel') ??
			ordered.find((r) => r.lang === 'rnx') ??
			ordered[0]
	);

	function speedup(r: Result): string {
		if (r.label === rnx.label) return 'baseline';
		const ratio = rnx.runtime_ms / r.runtime_ms;
		if (ratio >= 1) return `${ratio.toFixed(2)}x faster`;
		return `${(1 / ratio).toFixed(1)}x slower`;
	}

	// Pareto skyline (min runtime, min y): sorted by x, strictly decreasing y.
	// Carried as indices into `ordered` so the path, the zone, and the dot
	// markers can never disagree about which point they mean.
	const frontierIdx = $derived.by(() => {
		const sorted = ordered.map((r, i) => ({ r, i })).sort((a, b) => a.r.runtime_ms - b.r.runtime_ms);
		const out: number[] = [];
		let best = Infinity;
		for (const { r, i } of sorted) {
			const y = yMode === 'ram' ? r.peak_rss_mb : r.build_ms;
			if (y < best) {
				best = y;
				out.push(i);
			}
		}
		return out;
	});
	const frontier = $derived(frontierIdx.map((i) => ordered[i]));

	const fastest = $derived([...ordered].sort((a, b) => a.runtime_ms - b.runtime_ms)[0]);
	const leanest = $derived([...ordered].sort((a, b) => a.peak_rss_mb - b.peak_rss_mb)[0]);
	const fastestBuild = $derived([...ordered].sort((a, b) => a.build_ms - b.build_ms)[0]);
	// Ranked within one mode: a dev build competing against an optimized
	// release binary is not a like-for-like race.
	const rnxRank = $derived(
		[...ordered]
			.filter((r) => r.mode === (mode === 'dev' ? 'dev' : 'rel'))
			.sort((a, b) => a.runtime_ms - b.runtime_ms)
			.findIndex((r) => r.lang === 'rnx') + 1
	);
	const rankPool = $derived(ordered.filter((r) => r.mode === (mode === 'dev' ? 'dev' : 'rel')));

	// Chart geometry + log scales. `shown` tweens toward targets on tab/mode change.
	const W = 680;
	const H = 430;
	const L = 58;
	const R = 18;
	const T = 18;
	const B = 46;
	const plotW = W - L - R;
	const plotH = H - T - B;

	interface View {
		pts: { x: number; y: number }[];
		dx0: number;
		dx1: number;
		dy0: number;
		dy1: number;
	}

	function computeTargets(): View {
		const xs = ordered.map((r) => Math.log10(r.runtime_ms));
		const ys = ordered.map((r) => Math.log10(yOf(r)));
		const [dx0, dx1] = niceExtent(xs);
		const [dy0, dy1] = niceExtent(ys);
		return {
			pts: ordered.map((r) => ({ x: Math.log10(r.runtime_ms), y: Math.log10(yOf(r)) })),
			dx0,
			dx1,
			dy0,
			dy1
		};
	}

	// Automatic data-driven log-domain boundaries: pad the data span, then
	// snap outward to clean 1/2/5 x 10^k stops so axes always hug the data
	// with legible limits, even when every value is identical.
	function snapDown(v: number): number {
		const e = Math.floor(v + 1e-9);
		for (const m of [5, 2, 1]) {
			const c = e + Math.log10(m);
			if (c <= v + 1e-9) return c;
		}
		return e - 1 + Math.log10(5);
	}
	function snapUp(v: number): number {
		const e = Math.floor(v + 1e-9);
		for (const m of [1, 2, 5]) {
			const c = e + Math.log10(m);
			if (c >= v - 1e-9) return c;
		}
		return e + 1;
	}
	function niceExtent(vals: number[]): [number, number] {
		let lo = Math.min(...vals);
		let hi = Math.max(...vals);
		const span = hi - lo;
		const pad = span > 0 ? Math.max(0.12 * span, 0.05) : 0.3;
		lo -= pad;
		hi += pad;
		return [snapDown(lo), snapUp(hi)];
	}

	let shown = $state<View | null>(null);
	let raf = 0;
	const reduced =
		typeof window !== 'undefined' &&
		window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

	$effect(() => {
		// niceExtent over an empty array yields +/-Infinity, so hold the
		// animation off until a benchmark has actually loaded.
		if (!sel) return;
		const t = computeTargets();
		const start = untrack(() => shown) ?? t;
		if (reduced) {
			shown = t;
			return;
		}
		const t0 = performance.now();
		cancelAnimationFrame(raf);
		const tick = (now: number) => {
			const k = Math.min(1, (now - t0) / 400);
			const e = 1 - Math.pow(1 - k, 3);
			const lerp = (a: number, b: number) => a + (b - a) * e;
			shown = {
				pts: t.pts.map((p, i) => ({ x: lerp(start.pts[i]?.x ?? p.x, p.x), y: lerp(start.pts[i]?.y ?? p.y, p.y) })),
				dx0: lerp(start.dx0, t.dx0),
				dx1: lerp(start.dx1, t.dx1),
				dy0: lerp(start.dy0, t.dy0),
				dy1: lerp(start.dy1, t.dy1)
			};
			if (k < 1) raf = requestAnimationFrame(tick);
		};
		raf = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(raf);
	});

	const sx = $derived((lx: number) => L + ((lx - (shown?.dx0 ?? 0)) / ((shown?.dx1 ?? 1) - (shown?.dx0 ?? 0))) * plotW);
	const sy = $derived(
		(ly: number) => T + plotH - ((ly - (shown?.dy0 ?? 0)) / ((shown?.dy1 ?? 1) - (shown?.dy0 ?? 0))) * plotH
	);

	interface Tick {
		v: number;
		label: string;
	}
	function scaleTicks(d0: number, d1: number): Tick[] {
		const out: Tick[] = [];
		for (let p = Math.ceil(d0 - 1e-9); p <= d1 + 1e-9; p++) out.push({ v: p, label: fmtTick(p) });
		if (Math.abs(d0 - Math.round(d0)) > 1e-6)
			out.push({ v: d0, label: fmtVal(Math.pow(10, d0)) });
		if (Math.abs(d1 - Math.round(d1)) > 1e-6)
			out.push({ v: d1, label: fmtVal(Math.pow(10, d1)) });
		return out;
	}
	function minorTicks(d0: number, d1: number): number[] {
		const out: number[] = [];
		for (let p = Math.floor(d0); p <= Math.ceil(d1); p++) {
			for (let m = 2; m <= 9; m++) {
				const v = p + Math.log10(m);
				if (v > d0 && v < d1) out.push(v);
			}
		}
		return out;
	}
	const xticks = $derived(shown ? scaleTicks(shown.dx0, shown.dx1) : []);
	const yticks = $derived(shown ? scaleTicks(shown.dy0, shown.dy1) : []);
	const xminors = $derived(shown ? minorTicks(shown.dx0, shown.dx1) : []);
	const yminors = $derived(shown ? minorTicks(shown.dy0, shown.dy1) : []);

	// Collision-aware label placement: greedy over candidate slots
	// (right / above-right / below-right / left, then wider offsets),
	// rejecting any slot whose box hits another label or a dot.
	interface Placement {
		dx: number;
		dy: number;
		anchor: 'start' | 'end';
		leader: boolean;
	}
	const placements = $derived.by(() => {
		const fallback: Placement = { dx: 11, dy: 4, anchor: 'start', leader: false };
		if (!shown) return ordered.map(() => fallback);
		const tgt = computeTargets().pts;
		const pts = ordered.map((r, i) => {
			const p = shown!.pts[i] ?? tgt[i];
			return {
				i,
				x: sx(p.x),
				y: sy(p.y),
				lang: r.label,
				rnx: r.lang === 'rnx' && r.mode === 'rel'
			};
		});
		const dots = pts.map((p) => ({ x: p.x, y: p.y, r: (p.rnx ? 7 : 5.5) + 2 }));
		const seq = [...pts].sort(
			(a, b) => (b.rnx ? 1 : 0) - (a.rnx ? 1 : 0) || a.x - b.x || a.y - b.y
		);
		const boxes: { x0: number; y0: number; x1: number; y1: number }[] = [];
		const out: Placement[] = pts.map(() => fallback);
		const hits = (b: { x0: number; y0: number; x1: number; y1: number }) => {
			for (const o of boxes) {
				if (b.x0 < o.x1 && b.x1 > o.x0 && b.y0 < o.y1 && b.y1 > o.y0) return true;
			}
			for (const d of dots) {
				const cx = Math.max(b.x0, Math.min(d.x, b.x1));
				const cy = Math.max(b.y0, Math.min(d.y, b.y1));
				if ((cx - d.x) * (cx - d.x) + (cy - d.y) * (cy - d.y) < d.r * d.r) return true;
			}
			return false;
		};
		for (const n of seq) {
			const w = 10 + n.lang.length * 6.8;
			const h = 13;
			const cands: Placement[] = [
				{ dx: 11, dy: 4, anchor: 'start', leader: false },
				{ dx: 11, dy: -10, anchor: 'start', leader: true },
				{ dx: 11, dy: 18, anchor: 'start', leader: true },
				{ dx: -9, dy: 4, anchor: 'end', leader: true },
				{ dx: 11, dy: -24, anchor: 'start', leader: true },
				{ dx: 11, dy: 32, anchor: 'start', leader: true }
			];
			let placed: Placement = cands[0];
			let ok = false;
			for (const c of cands) {
				const x0 = c.anchor === 'start' ? n.x + c.dx : n.x + c.dx - w;
				const box = { x0, y0: n.y + c.dy - 10, x1: x0 + w, y1: n.y + c.dy + 3 };
				if (!hits(box)) {
					boxes.push(box);
					placed = c;
					ok = true;
					break;
				}
			}
			if (!ok) {
				const x0 = n.x + placed.dx;
				boxes.push({ x0, y0: n.y + placed.dy - 10, x1: x0 + w, y1: n.y + placed.dy + 3 });
			}
			out[n.i] = placed;
		}
		return out;
	});

	// Chart labels read "<lang> <mode>" from data; `rel` is a build mode,
	// not the `rnx rel` subcommand (which does not exist). Display it long.
	function displayLabel(label: string): string {
		return label.endsWith(' rel') ? `${label.slice(0, -4)} release` : label;
	}
	function fmtTick(p: number): string {
		const v = Math.pow(10, p);
		if (v >= 1000) return `${+(v / 1000).toFixed(v % 1000 === 0 ? 0 : 1)}k`;
		return `${v}`;
	}
	function fmtVal(v: number): string {
		if (v >= 1000) {
			const k = v / 1000;
			return `${+k.toFixed(k >= 100 ? 0 : 1)}k`;
		}
		if (v >= 100) return `${Math.round(v)}`;
		if (v >= 10) return `${+v.toFixed(1)}`;
		if (v >= 1) return `${+v.toFixed(2)}`;
		return `${+v.toFixed(3)}`;
	}

	const frontierPath = $derived.by(() => {
		if (!shown) return '';
		const tgt = computeTargets().pts;
		return frontierIdx
			.map((i) => shown!.pts[i] ?? tgt[i])
			.filter((p): p is { x: number; y: number } => !!p)
			.map((p) => `${sx(p.x).toFixed(1)},${sy(p.y).toFixed(1)}`)
			.join(' L ');
	});

	const idealRect = $derived.by(() => {
		if (!shown || frontierIdx.length === 0) return null;
		const tgt = computeTargets().pts;
		const fps = frontierIdx.map((i) => shown!.pts[i] ?? tgt[i]);
		const fx = Math.max(...fps.map((p) => sx(p.x)));
		// Clamp the zone to the lower 30% of the active Y scale so it
		// never stretches to the top of the domain.
		const bandTop = T + plotH * 0.7;
		const bottom = T + plotH;
		return { x: L, y: bandTop, w: Math.max(0, fx - L), h: Math.max(0, bottom - bandTop) };
	});

	const hoveredResult = $derived(ordered.find((r) => r.label === hovered) ?? null);
	const hoveredPt = $derived.by(() => {
		if (!hoveredResult || !shown) return null;
		const i = ordered.findIndex((r) => r.label === hoveredResult.label);
		const p = shown.pts[i] ?? computeTargets().pts[i];
		return p ? { x: sx(p.x), y: sy(p.y) } : null;
	});
</script>

<div class="bay" data-testid="pareto-scatter">
	{#if noData}
		<div class="px-4 py-8 md:px-8">
			<h3 class="text-lg font-bold text-aura-text md:text-xl">No benchmark data yet</h3>
			<p class="mt-2 max-w-prose font-mono text-[12px] leading-relaxed text-aura-muted">
				Rasmalai has not published a benchmark run yet, so this chart appears after the first
				monthly run.
			</p>
		</div>
	{:else if !sel}
		<div class="flex items-center gap-2 px-4 py-8 font-mono text-[12px] text-aura-muted">
			<LoaderCircle size={13} class="animate-spin" />Loading benchmark data...
		</div>
	{/if}

	{#if sel}
		<div
			class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-aura-border px-4 py-2 font-mono text-[11px]"
		>
			<span class="text-aura-muted">proving ground - measured runtime - {measuredOn}</span>
			<span class="ml-auto hidden text-aura-muted sm:inline">[ RUN: {measuredAt} ]</span>
		</div>

		<!-- benchmark tabs -->
		<div
			class="flex gap-1 overflow-x-auto border-b border-aura-border p-2"
			role="group"
			aria-label="Workload selector"
		>
			{#each BENCHES as b}
				<button
					onclick={() => (selId = b.id)}
					aria-pressed={selId === b.id}
					class="press shrink-0 rounded-full border px-3.5 py-1.5 font-mono text-[12px] whitespace-nowrap {selId ===
					b.id
						? 'border-aura-purple/50 bg-aura-purple/10 text-aura-purple'
						: 'border-aura-border text-aura-muted hover:border-aura-borderHover hover:text-aura-text'}"
				>
					{b.bench.name}
				</button>
			{/each}
		</div>

		{#key selId}
			<div use:reveal class="px-4 py-6 md:px-8">
				<div class="flex flex-wrap items-end gap-x-8 gap-y-1">
					<div>
						<h3 class="text-lg font-bold text-aura-text md:text-xl">{sel.name}</h3>
						<p class="font-mono text-[11px] text-aura-muted">
							{sel.description} - bottom-left is better
						</p>
					</div>
					<div class="tabular ml-auto flex items-baseline gap-5">
						<p class="font-mono text-3xl font-bold text-aura-green md:text-4xl">
							<CountUp value={rnx.runtime_ms} decimals={1} suffix=" ms" />
						</p>
						<p class="font-mono text-sm text-aura-muted">
							<CountUp value={rnx.peak_rss_mb} decimals={1} suffix=" MB peak" />
						</p>
					</div>
				</div>

				<!-- axis toolbar: what the chart maps, separate from workload tabs -->
				<div class="mt-4 flex flex-wrap items-center gap-x-4 gap-y-2">
					<span class="font-mono text-[10px] tracking-[0.2em] text-aura-muted uppercase"
						>x-axis - runtime (log scale)</span
					>
					<span class="flex items-center gap-2">
						<span class="font-mono text-[10px] tracking-[0.2em] text-aura-muted uppercase">y-axis</span>
						<span
							class="flex divide-x divide-aura-border overflow-hidden rounded-full border border-aura-border"
							role="group"
							aria-label="Y axis metric"
						>
							{#each [['ram', 'Peak RAM'], ['build', 'Build Time']] as [m, label]}
								<button
									onclick={() => (yMode = m as 'ram' | 'build')}
									aria-pressed={yMode === m}
									class="press px-3.5 py-1.5 font-mono text-[11px] whitespace-nowrap {yMode === m
										? 'bg-aura-green/10 text-aura-green'
										: 'text-aura-muted hover:text-aura-text'}"
								>
									{label}
								</button>
							{/each}
						</span>
					</span>
					<span class="flex items-center gap-2">
						<span class="font-mono text-[10px] tracking-[0.2em] text-aura-muted uppercase">modes</span>
						<span
							class="flex divide-x divide-aura-border overflow-hidden rounded-full border border-aura-border"
							role="group"
							aria-label="Build mode filter"
						>
							{#each [['all', 'All'], ['rel', 'Release'], ['dev', 'Dev']] as [m, label]}
								<button
									onclick={() => (mode = m as 'all' | 'dev' | 'rel')}
									aria-pressed={mode === m}
									class="press px-3.5 py-1.5 font-mono text-[11px] whitespace-nowrap {mode === m
										? 'bg-aura-green/10 text-aura-green'
										: 'text-aura-muted hover:text-aura-text'}"
								>
									{label}
								</button>
							{/each}
						</span>
					</span>
				</div>

				<p class="mt-2 font-mono text-[11px] text-aura-muted">
					{#if mode === 'all'}
						Filled dots are release builds, hollow dots are dev builds. Languages with a
						real dev/release switch appear twice; go, node, and java compile once, so they
						appear once.
					{:else if mode === 'rel'}
						Release builds only, the configuration a shipped binary runs.
					{:else}
						Dev builds only: unoptimized codegen and JIT, the configuration you edit
						against.
					{/if}
				</p>

				<div class="mt-3 flex flex-wrap gap-x-6 gap-y-1 font-mono text-[11px] text-aura-muted">
					<span class="inline-flex items-center gap-2">
						<svg width="18" height="12" viewBox="0 0 18 12" aria-hidden="true">
							<rect
								x="1"
								y="1"
								width="16"
								height="10"
								fill="#61ffca"
								opacity="0.12"
								stroke="#61ffca"
								stroke-opacity="0.65"
								stroke-dasharray="3 2"
							/>
						</svg>
						<span><strong class="font-semibold text-aura-text">Ideal zone:</strong> lower-left corner where runtime and {yMetric} are both low.</span>
					</span>
					<span class="inline-flex items-center gap-2">
						<svg width="18" height="12" viewBox="0 0 18 12" aria-hidden="true">
							<line x1="1" y1="11" x2="17" y2="11" stroke="#6d6d6d" stroke-width="1" />
							<line x1="6" y1="2" x2="6" y2="11" stroke="#6d6d6d" stroke-width="1" />
							<line x1="12" y1="2" x2="12" y2="11" stroke="#6d6d6d" stroke-width="1" />
						</svg>
						<span><strong class="font-semibold text-aura-text">Log scale:</strong> major gridlines multiply by 10, so equal spacing is not an equal difference.</span>
					</span>
				</div>

				<!-- scatter chart (desktop) -->
				<div class="relative mt-3 hidden md:block">
					<svg
						viewBox="0 0 {W} {H}"
						class="w-full"
						role="img"
						aria-label="{sel.name}: runtime versus {yMetric} by language and build mode"
					>
						<defs>
							<marker
								id="pareto-axis-arrow"
								viewBox="0 0 10 10"
								refX="8"
								refY="5"
								markerWidth="7"
								markerHeight="7"
								orient="auto-start-reverse"
							>
								<path d="M0,1 L9,5 L0,9 z" fill="#6d6d6d" />
							</marker>
						</defs>
						{#if shown && idealRect}
							<rect
								x={idealRect.x}
								y={idealRect.y}
								width={idealRect.w}
								height={idealRect.h}
								fill="#61ffca"
								opacity="0.045"
							/>
							<rect
								x={idealRect.x}
								y={idealRect.y}
								width={idealRect.w}
								height={idealRect.h}
								fill="none"
								stroke="#61ffca"
								stroke-opacity="0.25"
								stroke-dasharray="4 4"
							/>
							<text
								x={idealRect.x + 6}
								y={idealRect.y + 14}
								fill="#61ffca"
								opacity="0.55"
								font-size="10"
								font-family="JetBrains Mono, Menlo, monospace">ideal zone</text
							>
						{/if}

						{#each xminors as v}
							<line
								x1={L + ((v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
								y1={T}
								x2={L + ((v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
								y2={T + plotH}
								stroke="rgba(255,255,255,0.03)"
							/>
						{/each}
						{#each yminors as v}
							<line
								x1={L}
								y1={T + plotH - ((v - shown!.dy0) / (shown!.dy1 - shown!.dy0)) * plotH}
								x2={L + plotW}
								y2={T + plotH - ((v - shown!.dy0) / (shown!.dy1 - shown!.dy0)) * plotH}
								stroke="rgba(255,255,255,0.03)"
							/>
						{/each}
						{#each xticks as t}
							<line
								x1={L + ((t.v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
								y1={T}
								x2={L + ((t.v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
								y2={T + plotH}
								stroke="rgba(255,255,255,0.07)"
							/>
							<text
								x={L + ((t.v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
								y={T + plotH + 18}
								fill="#6d6d6d"
								font-size="10"
								text-anchor="middle"
								font-family="JetBrains Mono, Menlo, monospace">{t.label}</text
							>
						{/each}
						{#each yticks as t}
							<line
								x1={L}
								y1={T + plotH - ((t.v - shown!.dy0) / (shown!.dy1 - shown!.dy0)) * plotH}
								x2={L + plotW}
								y2={T + plotH - ((t.v - shown!.dy0) / (shown!.dy1 - shown!.dy0)) * plotH}
								stroke="rgba(255,255,255,0.07)"
							/>
							<text
								x={L - 8}
								y={T + plotH - ((t.v - shown!.dy0) / (shown!.dy1 - shown!.dy0)) * plotH + 3}
								fill="#6d6d6d"
								font-size="10"
								text-anchor="end"
								font-family="JetBrains Mono, Menlo, monospace">{t.label}</text
							>
						{/each}
						<rect
							x={L}
							y={T}
							width={plotW}
							height={plotH}
							fill="none"
							stroke="rgba(255,255,255,0.12)"
							stroke-width="1"
						/>
						<line
							x1={L}
							y1={T + plotH}
							x2={L + plotW - 1}
							y2={T + plotH}
							stroke="#6d6d6d"
							stroke-width="1"
							marker-end="url(#pareto-axis-arrow)"
						/>
						<line
							x1={L}
							y1={T + plotH}
							x2={L}
							y2={T + 1}
							stroke="#6d6d6d"
							stroke-width="1"
							marker-end="url(#pareto-axis-arrow)"
						/>

						<text
							x={L + plotW / 2}
							y={H - 6}
							fill="#6d6d6d"
							font-size="10"
							text-anchor="middle"
							font-family="JetBrains Mono, Menlo, monospace">runtime (ms, log scale)</text
						>
						<text
							x={14}
							y={T + plotH / 2}
							fill="#6d6d6d"
							font-size="10"
							text-anchor="middle"
							transform="rotate(-90 14 {T + plotH / 2})"
							font-family="JetBrains Mono, Menlo, monospace">{yLabel}</text
						>

						{#if frontierPath}
							<path
								d="M {frontierPath}"
								fill="none"
								stroke="#61ffca"
								stroke-opacity="0.4"
								stroke-dasharray="5 5"
								stroke-width="1.5"
							/>
							{#each frontierIdx as fi}
								{@const fp = shown!.pts[fi] ?? computeTargets().pts[fi]}
								{#if fp}
									<circle
										cx={sx(fp.x)}
										cy={sy(fp.y)}
										r="2.5"
										fill="#61ffca"
										opacity="0.85"
										stroke="#15141b"
										stroke-width="1"
									/>
								{/if}
							{/each}
						{/if}

						{#if shown}
							{#each ordered as r, i}
								{@const pt = shown.pts[i] ?? computeTargets().pts[i]}
								{#if pt}
									{@const cx = sx(pt.x)}
									{@const cy = sy(pt.y)}
									{@const isRnx = r.lang === 'rnx' && r.mode === 'rel'}
									{@const isHover = hovered === r.label}
									{@const place = placements[i] ?? { dx: 11, dy: 4, anchor: 'start', leader: false }}
									<g
										role="button"
										tabindex="0"
										aria-label="{displayLabel(r.label)}: {r.runtime_ms} ms, {speedup(r)}"
										onmouseenter={() => (hovered = r.label)}
										onmouseleave={() => (hovered = null)}
										onfocus={() => (hovered = r.label)}
										onblur={() => (hovered = null)}
										style="cursor: crosshair"
									>
										{#if isRnx}
											<circle cx={cx} cy={cy} r="9" fill="none" stroke="#61ffca" class="pulse-ring" />
										{/if}
										<circle
											cx={cx}
											cy={cy}
											r={isRnx ? 7 : isHover ? 7 : 5.5}
											fill={r.mode === 'dev' ? '#15141b' : LANG_COLOR[r.lang] ?? '#8b9bb4'}
											opacity={isRnx || isHover ? 1 : 0.8}
											stroke={LANG_COLOR[r.lang] ?? '#8b9bb4'}
											stroke-width={r.mode === 'dev' ? 2 : 1.5}
											stroke-opacity={r.mode === 'dev' ? 0.9 : 1}
											style={isRnx
												? 'filter: drop-shadow(0 0 6px rgba(97,255,202,0.8))'
												: undefined}
										/>
										<title>{displayLabel(r.label)}</title>
										{#if place.leader}
											<line
												x1={cx + (place.anchor === 'start' ? 5 : -5)}
												y1={cy}
												x2={cx + place.dx + (place.anchor === 'start' ? -1 : 1)}
												y2={cy + place.dy - 3}
												stroke={LANG_COLOR[r.lang] ?? '#8b9bb4'}
												stroke-opacity="0.4"
												stroke-width="1"
											/>
										{/if}
										<text
											x={cx + place.dx}
											y={cy + place.dy}
											text-anchor={place.anchor === 'start' ? 'start' : 'end'}
											fill={isRnx ? '#61ffca' : '#edecee'}
											opacity={isRnx || isHover ? 1 : 0.75}
											font-size="11"
											font-weight={isRnx ? 'bold' : 'normal'}
											font-family="JetBrains Mono, Menlo, monospace"
											style="paint-order: stroke; stroke: #15141b; stroke-width: 3px;">{displayLabel(r.label)}</text
										>
									</g>
								{/if}
							{/each}
						{/if}
					</svg>

					{#if hoveredResult && hoveredPt}
						{@const flip = hoveredPt.x > W * 0.62}
						<div
							class="pointer-events-none absolute z-10 w-60 rounded border border-aura-border bg-aura-surfaceElevated p-3 font-mono text-[11px] shadow-xl"
							style="left: {(flip ? hoveredPt.x - 250 : hoveredPt.x + 16) / W * 100}%; top: {Math.max(
								0,
								(hoveredPt.y - 40) / H * 100
							)}%"
						>
							<p class="text-xs font-bold" style="color: {LANG_COLOR[hoveredResult.lang]}">
								{hoveredResult.label}
							</p>
							<p class="mt-0.5 truncate text-aura-muted">{versions[hoveredResult.lang] ?? ''}</p>
							<p class="mt-1.5 text-aura-text">
								{hoveredResult.runtime_ms.toFixed(1)} ms - {speedup(hoveredResult)}
							</p>
							<p class="text-aura-muted">{hoveredResult.peak_rss_mb.toFixed(1)} MB peak RAM</p>
							<p class="text-aura-muted">
								{hoveredResult.build_ms.toFixed(1)} ms {hoveredResult.artifact
									? 'build'
									: 'check (no build step)'}
							</p>
							<p class="mt-1.5 leading-relaxed text-aura-muted">build - {hoveredResult.build}</p>
							<p class="text-aura-muted/70">run - {hoveredResult.run}</p>
						</div>
					{/if}
				</div>

				<!-- condensed cards (mobile) -->
				<div class="mt-5 space-y-2 md:hidden">
					{#each [...ordered].sort((a, b) => a.runtime_ms - b.runtime_ms) as r}
						<div
							class="flex items-center gap-3 rounded border px-3 py-2 font-mono text-[11px] {r.lang ===
							'rnx' && r.mode === 'rel'
								? 'border-aura-green/40 bg-aura-green/5'
								: 'border-aura-border'}"
						>
							<span
								class="inline-block h-2.5 w-2.5 shrink-0 rounded-full {r.mode === 'dev'
									? 'border-2 bg-transparent'
									: ''}"
								style="border-color: {LANG_COLOR[r.lang]}; background: {r.mode === 'dev'
									? 'transparent'
									: LANG_COLOR[r.lang]}"
							></span>
							<span
								class="w-16 {r.lang === 'rnx' && r.mode === 'rel'
									? 'font-bold text-aura-green'
									: 'text-aura-text'}">{displayLabel(r.label)}</span
							>
							<span class="ml-auto text-right text-aura-muted">
								<span class="text-aura-text">{r.runtime_ms.toFixed(1)} ms</span>
								{' - '}{r.peak_rss_mb.toFixed(1)} MB - {r.build_ms.toFixed(1)} ms build
							</span>
						</div>
					{/each}
				</div>

				<!-- summary strip -->
				<div class="mt-5 grid grid-cols-1 gap-2 font-mono text-[11px] sm:grid-cols-3">
					<div class="rounded border border-aura-border px-3 py-2">
						<p class="text-aura-muted">fastest runtime</p>
						<p class="mt-0.5 text-sm text-aura-text">
							{fastest.label} - {fastest.runtime_ms.toFixed(1)} ms
						</p>
					</div>
					<div class="rounded border border-aura-border px-3 py-2">
						<p class="text-aura-muted">
							{yMode === 'ram'
								? 'lowest memory'
								: fastestBuild.artifact
									? 'fastest build'
									: 'fastest check (no build step)'}
						</p>
						<p class="mt-0.5 text-sm text-aura-text">
							{#if yMode === 'ram'}
								{leanest.label} - {leanest.peak_rss_mb.toFixed(1)} MB
							{:else}
								{fastestBuild.label} - {fastestBuild.build_ms.toFixed(1)} ms
							{/if}
						</p>
					</div>
					<div class="rounded border border-aura-green/40 bg-aura-green/5 px-3 py-2">
						<p class="text-aura-muted">rasmalai</p>
						<p class="mt-0.5 text-sm text-aura-green">
							#{rnxRank} of {rankPool.length} in {mode === 'dev' ? 'dev' : 'release'} - {rnx.runtime_ms.toFixed(
								1
							)} ms - {rnx.peak_rss_mb.toFixed(1)} MB
						</p>
					</div>
				</div>

				<details class="mt-4 font-mono text-[11px] text-aura-muted">
					<summary class="cursor-pointer hover:text-aura-text">how this was measured</summary>
					<p class="mt-2 leading-relaxed">
						Median of 5 timed runs after 1 warm-up, plus a median of 3 builds after 1
						warm-up for each configuration. Peak RSS is the benchmark process's own (a
						~16 KB native helper fork/execvps the target and reports the child's
						getrusage, so no harness footprint leaks in). Every row is one (language,
						mode) pair: release is the shipping build, dev is the unoptimized or JIT
						build you edit against. go, node, and java have a single configuration
						each, so they contribute one row - go always compiles optimized, node is
						interpreted with no build step (its timed step is a parse check, flagged as
						such), and javac has no optimization flag. C float benches build with
						-ffp-contract=off for strict-IEEE parity. Checksums verified bit-for-bit
						(1e-6 relative tolerance on floats) so every configuration produced the
						same answer; implementations differ per language (see benches/README.md),
						so rows rank whole-stack cost, not language speed alone. Reproduce locally:
						<code>python3 benches/harness/runner.py</code>.
					</p>
					<ul class="mt-2 space-y-0.5">
						{#each sel.results as r}
							<li>
								<span class="text-aura-text/70">{displayLabel(r.label)}</span> - {versions[r.lang] ?? ''} ·
								<span class="text-aura-text/50">{r.build}</span>
								<span class="text-aura-text/50"> · run: {r.run}</span>
							</li>
						{/each}
					</ul>
				</details>
			</div>
		{/key}
	{/if}
</div>

<style>
	.pulse-ring {
		opacity: 0.7;
		transform-box: fill-box;
		transform-origin: center;
		animation: pulse 2s ease-out infinite;
	}
	@keyframes pulse {
		0% {
			transform: scale(0.7);
			opacity: 0.8;
		}
		70% {
			transform: scale(1.6);
			opacity: 0;
		}
		100% {
			transform: scale(1.6);
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.pulse-ring {
			animation: none;
		}
	}
</style>

