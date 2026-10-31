<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { LoaderCircle } from 'lucide-svelte';
	import CountUp from '$lib/motion/CountUp.svelte';

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
		rnx: 'rgb(var(--aura-green))',
		c: 'rgb(var(--aura-faint))',
		rust: 'rgb(var(--aura-orange))',
		go: 'rgb(var(--aura-cyan))',
		node: 'rgb(var(--aura-green))',
		java: 'rgb(var(--aura-red))',
		dart: 'rgb(var(--aura-pink))'
	};

	const measuredOn = $derived(snapshot?.system?.cpu ?? 'unknown machine');
	const measuredAt = $derived((snapshot?.system?.date ?? '').slice(0, 10));
	const versions = $derived(snapshot?.versions ?? {});

	let selId = $state('');
	let yMode = $state<'ram' | 'build'>('ram');
	let mode = $state<'all' | 'dev' | 'rel'>('all');
	let hovered = $state<string | null>(null);
	let pinned = $state<string | null>(null);

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

	// Label placement: every dot is named, anchored right, staggered so no
	// two labels share a row and no label covers a dot. Near the right
	// edge the label flips left.
	const labelLayout = $derived.by((): { dx: number; dy: number; anchor: 'start' | 'end' }[] => {
		const idle = { dx: 11, dy: 4, anchor: 'start' as const };
		if (!shown) return ordered.map(() => ({ ...idle }));
		const tgt = computeTargets().pts;
		const items = ordered.map((r, i) => {
			const p = shown!.pts[i] ?? tgt[i];
			return { i, x: p ? sx(p.x) : 0, y: p ? sy(p.y) : 0, w: 10 + displayLabel(r.label).length * 6.8 };
		});
		// Dots are blockers too: a label may not cover any dot but its own.
		const dots = items.map((d) => ({ x0: d.x - 10, x1: d.x + 10, y0: d.y - 10, y1: d.y + 10 }));
		const placed: { x0: number; x1: number; y0: number; y1: number }[] = [];
		const out: { dx: number; dy: number; anchor: 'start' | 'end' }[] = items.map(() => ({
			dx: 11,
			dy: 4,
			anchor: 'start' as const
		}));
		for (const { i, x, y, w } of [...items].sort((a, b) => a.y - b.y)) {
			const flips = x > W - 130;
			const anchor = flips ? 'end' : 'start';
			const x0 = flips ? x - 9 - w : x + 11;
			const x1 = flips ? x - 9 : x + 11 + w;
			let dy = 4;
			for (let round = 0; round < 12; round++) {
				const y0 = y + dy - 11;
				const y1 = y + dy + 4;
				const hit = [...dots.filter((_, di) => di !== i), ...placed].find(
					(b) => x0 < b.x1 && x1 > b.x0 && y0 < b.y1 && y1 > b.y0
				);
				if (!hit) {
					placed.push({ x0, x1, y0, y1 });
					break;
				}
				dy = hit.y1 - y + 12;
				if (round === 11) placed.push({ x0, x1, y0: y + dy - 11, y1: y + dy + 4 });
			}
			out[i] = { dx: flips ? -9 : 11, dy, anchor };
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

	const hoveredResult = $derived(ordered.find((r) => r.label === hovered) ?? null);
	const pinnedResult = $derived(ordered.find((r) => r.label === pinned) ?? null);
	const tipResult = $derived(hoveredResult ?? pinnedResult);
	const tipPt = $derived.by(() => {
		if (!tipResult || !shown) return null;
		const i = ordered.findIndex((r) => r.label === tipResult.label);
		const p = shown.pts[i] ?? computeTargets().pts[i];
		return p ? { x: sx(p.x), y: sy(p.y) } : null;
	});

	function togglePin(label: string) {
		pinned = pinned === label ? null : label;
	}

	function onDotKeydown(event: KeyboardEvent, label: string) {
		if (event.key === 'Enter' || event.key === ' ') {
			event.preventDefault();
			togglePin(label);
		} else if (event.key === 'Escape') {
			pinned = null;
		}
	}
</script>

<div class="speed" data-testid="pareto-scatter">
	{#if noData}
		<p class="speed-empty">No benchmark data yet. Rasmalai has not published a benchmark run, so this figure appears after the first monthly run.</p>
	{:else if !sel}
		<p class="speed-empty speed-loading">
			<LoaderCircle size={13} class="motion-reduce:animate-none animate-spin" />Loading benchmark data...
		</p>
	{/if}

	{#if sel}
		<p class="speed-prov">Measured runtime on {measuredOn} · run {measuredAt}</p>

		<div class="speed-tabs" role="group" aria-label="Workload selector">
			{#each BENCHES as b, i}
				<button
					onclick={() => (selId = b.id)}
					aria-pressed={selId === b.id}
					class="speed-tab"
					class:speed-tab-active={selId === b.id}
				>
					<span class="speed-index">{String(i + 1).padStart(2, '0')}</span>
					{b.bench.name}
				</button>
			{/each}
		</div>

		{#key selId}
			<div class="speed-head">
				<div class="speed-head-main">
					<h3 class="speed-title">{sel.name}</h3>
					<p class="speed-desc">{sel.description}, bottom-left is better</p>
				</div>
				<div class="speed-head-figures">
					<p class="speed-figure">
						<CountUp value={rnx.runtime_ms} decimals={1} suffix=" ms" />
					</p>
					<p class="speed-sub"><CountUp value={rnx.peak_rss_mb} decimals={1} suffix=" MB peak" /></p>
				</div>
			</div>

			<div class="speed-controls">
				<span class="speed-group" role="group" aria-label="Y axis metric">
					<span class="speed-group-cap">against</span>
					{#each [['ram', 'peak RAM'], ['build', 'build time']] as [m, label]}
						<button
							onclick={() => (yMode = m as 'ram' | 'build')}
							aria-pressed={yMode === m}
							class="speed-toggle"
							class:speed-toggle-active={yMode === m}
						>
							{label}
						</button>
					{/each}
				</span>
				<span class="speed-group" role="group" aria-label="Build mode filter">
					<span class="speed-group-cap">builds</span>
					{#each [['all', 'all'], ['rel', 'release'], ['dev', 'dev']] as [m, label]}
						<button
							onclick={() => (mode = m as 'all' | 'dev' | 'rel')}
							aria-pressed={mode === m}
							class="speed-toggle"
							class:speed-toggle-active={mode === m}
						>
							{label}
						</button>
					{/each}
				</span>
				<span class="speed-key">filled is release, hollow is dev, both axes log</span>
			</div>

			<div class="speed-plot hidden md:block">
				<svg
					viewBox="0 0 {W} {H}"
					class="w-full"
					role="group"
					aria-label="{sel.name}: runtime versus {yMetric} by language and build mode"
				>
					{#each xminors as v}
						<line
							x1={L + ((v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
							y1={T}
							x2={L + ((v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
							y2={T + plotH}
							stroke="rgb(var(--aura-text) / 0.05)"
						/>
					{/each}
					{#each yminors as v}
						<line
							x1={L}
							y1={T + plotH - ((v - shown!.dy0) / (shown!.dy1 - shown!.dy0)) * plotH}
							x2={L + plotW}
							y2={T + plotH - ((v - shown!.dy0) / (shown!.dy1 - shown!.dy0)) * plotH}
							stroke="rgb(var(--aura-text) / 0.05)"
						/>
					{/each}
					{#each xticks as t}
						<line
							x1={L + ((t.v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
							y1={T}
							x2={L + ((t.v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
							y2={T + plotH}
							stroke="rgb(var(--aura-text) / 0.1)"
						/>
						<text
							x={L + ((t.v - shown!.dx0) / (shown!.dx1 - shown!.dx0)) * plotW}
							y={T + plotH + 18}
							fill="rgb(var(--aura-faint))"
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
							stroke="rgb(var(--aura-text) / 0.1)"
						/>
						<text
							x={L - 8}
							y={T + plotH - ((t.v - shown!.dy0) / (shown!.dy1 - shown!.dy0)) * plotH + 3}
							fill="rgb(var(--aura-faint))"
							font-size="10"
							text-anchor="end"
							font-family="JetBrains Mono, Menlo, monospace">{t.label}</text
						>
					{/each}
					<line
						x1={L}
						y1={T + plotH}
						x2={L + plotW}
						y2={T + plotH}
						stroke="rgb(var(--aura-faint))"
						stroke-width="1"
					/>
					<line
						x1={L}
						y1={T + plotH}
						x2={L}
						y2={T}
						stroke="rgb(var(--aura-faint))"
						stroke-width="1"
					/>

					<text
						x={L + plotW / 2}
						y={H - 6}
						fill="rgb(var(--aura-faint))"
						font-size="10"
						text-anchor="middle"
						font-family="JetBrains Mono, Menlo, monospace">runtime (ms, log scale)</text
					>
					<text
						x={14}
						y={T + plotH / 2}
						fill="rgb(var(--aura-faint))"
						font-size="10"
						text-anchor="middle"
						transform="rotate(-90 14 {T + plotH / 2})"
						font-family="JetBrains Mono, Menlo, monospace">{yLabel}</text
					>

					{#if shown}
						{#each ordered as r, i}
							{@const pt = shown.pts[i] ?? computeTargets().pts[i]}
							{#if pt}
								{@const cx = sx(pt.x)}
								{@const cy = sy(pt.y)}
								{@const isRnx = r.lang === 'rnx' && r.mode === 'rel'}
								{@const isActive = hovered === r.label || pinned === r.label}
								{@const lay = labelLayout[i] ?? { dx: 11, dy: 4, anchor: 'start' as const }}
								<g
									class="dot"
									role="button"
									tabindex="0"
									aria-pressed={pinned === r.label}
									aria-label="{displayLabel(r.label)}: {r.runtime_ms} ms, {speedup(r)}"
									onmouseenter={() => (hovered = r.label)}
									onmouseleave={() => (hovered = null)}
									onfocus={() => (hovered = r.label)}
									onblur={() => (hovered = null)}
									onclick={() => togglePin(r.label)}
									onkeydown={(event) => onDotKeydown(event, r.label)}
									style="cursor: crosshair"
								>
									{#if isRnx || isActive}
										<circle cx={cx} cy={cy} r="10" fill="none" stroke="rgb(var(--aura-green))" stroke-opacity="0.45" />
									{/if}
									<circle
										cx={cx}
										cy={cy}
										r={isRnx ? 7 : isActive ? 7 : 5.5}
										fill={r.mode === 'dev' ? 'rgb(var(--aura-bg))' : LANG_COLOR[r.lang] ?? 'rgb(var(--aura-faint))'}
										opacity={isRnx || isActive ? 1 : 0.85}
										stroke={LANG_COLOR[r.lang] ?? 'rgb(var(--aura-faint))'}
										stroke-width={r.mode === 'dev' ? 2 : 1.5}
										stroke-opacity={r.mode === 'dev' ? 0.9 : 1}
										style={isRnx
											? 'filter: drop-shadow(0 0 6px rgb(var(--aura-green) / 0.8))'
											: undefined}
									/>
									<title>{displayLabel(r.label)}</title>
									{#if lay.dy > 16}
										<line
											x1={cx + (lay.anchor === 'start' ? 6 : -6)}
											y1={cy}
											x2={cx + lay.dx}
											y2={cy + lay.dy - 3}
											stroke={LANG_COLOR[r.lang] ?? 'rgb(var(--aura-faint))'}
											stroke-opacity="0.35"
											stroke-width="1"
										/>
									{/if}
									<text
										x={cx + lay.dx}
										y={cy + lay.dy}
										text-anchor={lay.anchor === 'start' ? 'start' : 'end'}
										fill={isRnx ? 'rgb(var(--aura-green))' : 'rgb(var(--aura-text))'}
										opacity={isRnx || isActive ? 1 : 0.75}
										font-size="11"
										font-weight={isRnx ? 'bold' : 'normal'}
										font-family="JetBrains Mono, Menlo, monospace"
										style="paint-order: stroke; stroke: rgb(var(--aura-bg)); stroke-width: 3px;">{displayLabel(r.label)}</text
									>
								</g>
							{/if}
						{/each}
					{/if}
				</svg>

				{#if tipResult && tipPt}
					{@const flip = tipPt.x > W * 0.62}
					<div
						class="speed-tip"
						style="left: {(flip ? tipPt.x - 250 : tipPt.x + 16) / W * 100}%; top: {Math.max(
							0,
							(tipPt.y - 40) / H * 100
						)}%"
					>
						<p class="speed-tip-name" style="color: {LANG_COLOR[tipResult.lang]}">
							{tipResult.label}
						</p>
						<p class="speed-tip-ver">{versions[tipResult.lang] ?? ''}</p>
						<p class="speed-tip-fig">
							{tipResult.runtime_ms.toFixed(1)} ms - {speedup(tipResult)}
						</p>
						<p>{tipResult.peak_rss_mb.toFixed(1)} MB peak RAM</p>
						<p>
							{tipResult.build_ms.toFixed(1)} ms {tipResult.artifact
								? 'build'
								: 'check (no build step)'}
						</p>
						<p class="speed-tip-cmd">build - {tipResult.build}</p>
						<p class="speed-tip-run">run - {tipResult.run}</p>
					</div>
				{/if}
			</div>

			<ol class="speed-rows md:hidden">
				{#each [...ordered].sort((a, b) => a.runtime_ms - b.runtime_ms) as r}
					<li
						class="speed-row"
						class:speed-row-rnx={r.lang === 'rnx' && r.mode === 'rel'}
					>
						<span
							class="speed-row-dot"
							class:speed-row-hollow={r.mode === 'dev'}
							style="border-color: {LANG_COLOR[r.lang]}; background: {r.mode === 'dev'
								? 'transparent'
								: LANG_COLOR[r.lang]}"
						></span>
						<span class="speed-row-name">{displayLabel(r.label)}</span>
						<span class="speed-row-nums">
							<span class="speed-row-fig">{r.runtime_ms.toFixed(1)} ms</span>
							{' - '}{r.peak_rss_mb.toFixed(1)} MB - {r.build_ms.toFixed(1)} ms build
						</span>
					</li>
				{/each}
			</ol>

			<dl class="speed-facts">
				<div class="speed-fact">
					<dt>fastest runtime</dt>
					<dd>{fastest.label} - {fastest.runtime_ms.toFixed(1)} ms</dd>
				</div>
				<div class="speed-fact">
					<dt>
						{yMode === 'ram'
							? 'lowest memory'
							: fastestBuild.artifact
								? 'fastest build'
								: 'fastest check (no build step)'}
					</dt>
					<dd>
						{#if yMode === 'ram'}
							{leanest.label} - {leanest.peak_rss_mb.toFixed(1)} MB
						{:else}
							{fastestBuild.label} - {fastestBuild.build_ms.toFixed(1)} ms
						{/if}
					</dd>
				</div>
				<div class="speed-fact">
					<dt>rasmalai</dt>
					<dd>
						#{rnxRank} of {rankPool.length} in {mode === 'dev' ? 'dev' : 'release'} - {rnx.runtime_ms.toFixed(
							1
						)} ms - {rnx.peak_rss_mb.toFixed(1)} MB
					</dd>
				</div>
			</dl>

			<details class="speed-how">
				<summary>how this was measured</summary>
				<p>
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
				<ul class="speed-cmps">
					{#each sel.results as r}
						<li>
							<span>{displayLabel(r.label)}</span>
							<span>{versions[r.lang] ?? ''}</span>
							<span>{r.build}</span>
							<span>run {r.run}</span>
						</li>
					{/each}
				</ul>
			</details>
		{/key}
	{/if}
</div>

<style>
	.speed {
		min-width: 0;
	}

	.speed-empty {
		padding: var(--space-lg) 0;
		font-size: var(--text-sm);
		line-height: 1.65;
		color: var(--color-muted);
	}

	.speed-loading {
		display: flex;
		align-items: center;
		gap: var(--space-xs);
	}

	.speed-prov {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.speed-tabs {
		display: flex;
		flex-wrap: wrap;
		gap: var(--space-xs) var(--space-lg);
		border-bottom: 1px solid var(--color-rule);
		padding-bottom: var(--space-sm);
		margin-top: var(--space-md);
	}

	.speed-tab {
		display: inline-flex;
		align-items: baseline;
		gap: var(--space-xs);
		font-size: var(--text-sm);
		color: var(--color-faint);
		white-space: nowrap;
		transition: color var(--dur-micro) ease;
	}

	.speed-tab:hover {
		color: var(--color-ink);
	}

	.speed-tab-active {
		color: var(--color-ink);
		text-decoration: underline;
		text-decoration-color: var(--color-accent);
		text-decoration-thickness: 1px;
		text-underline-offset: 7px;
	}

	.speed-index {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.speed-head {
		display: flex;
		flex-wrap: wrap;
		align-items: end;
		justify-content: space-between;
		gap: var(--space-md) var(--space-xl);
		padding-top: var(--space-xl);
	}

	.speed-head-main {
		min-width: 0;
	}

	.speed-title {
		font-family: var(--font-display);
		font-weight: 600;
		font-size: clamp(1.35rem, 1.7vw, 1.7rem);
		line-height: 1.08;
		letter-spacing: -0.02em;
		color: var(--color-ink);
	}

	.speed-desc {
		margin-top: var(--space-2xs);
		font-size: var(--text-xs);
		color: var(--color-faint);
	}

	.speed-head-figures {
		display: flex;
		align-items: baseline;
		gap: var(--space-md);
	}

	.speed-figure {
		font-family: var(--font-display);
		font-weight: 600;
		font-size: clamp(2rem, 3vw, 2.6rem);
		line-height: 1;
		font-variant-numeric: tabular-nums;
		color: var(--color-signal);
	}

	.speed-sub {
		font-family: var(--font-mono);
		font-size: var(--text-sm);
		color: var(--color-muted);
	}

	.speed-controls {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: var(--space-xs) var(--space-xl);
		margin-top: var(--space-md);
	}

	.speed-group {
		display: inline-flex;
		align-items: baseline;
		gap: var(--space-sm);
	}

	.speed-group-cap {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.speed-toggle {
		font-size: var(--text-sm);
		color: var(--color-faint);
		transition: color var(--dur-micro) ease;
	}

	.speed-toggle:hover {
		color: var(--color-ink);
	}

	.speed-toggle-active {
		color: var(--color-ink);
		text-decoration: underline;
		text-decoration-color: var(--color-accent);
		text-decoration-thickness: 1px;
		text-underline-offset: 5px;
	}

	.speed-key {
		margin-top: var(--space-sm);
		font-size: var(--text-xs);
		color: var(--color-faint);
	}

	.speed-plot {
		position: relative;
		margin-top: var(--space-md);
	}

	/* Keyboard focus lands on the same halo a hovered dot gets: the dot
	grows and the tooltip opens, so focus is never a silent state. */
	.speed g.dot:focus {
		outline: none;
	}

	.speed g.dot:focus-visible {
		outline: 2px solid var(--color-accent);
		outline-offset: 4px;
	}

	.speed-tip {
		position: absolute;
		z-index: 10;
		width: 15rem;
		pointer-events: none;
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-xs);
		background: var(--color-paper);
		padding: var(--space-sm);
		font-family: var(--font-mono);
		font-size: 11px;
		line-height: 1.6;
		color: var(--color-muted);
	}

	.speed-tip-name {
		font-size: var(--text-xs);
		font-weight: 600;
		color: var(--color-ink);
	}

	.speed-tip-ver {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.speed-tip-fig {
		margin-top: var(--space-2xs);
		color: var(--color-ink);
	}

	.speed-tip-cmd {
		margin-top: var(--space-2xs);
	}

	.speed-tip-run {
		color: rgb(var(--aura-text) / 0.6);
	}

	.speed-rows {
		list-style: none;
		margin: var(--space-lg) 0 0;
		padding: 0;
	}

	.speed-row {
		display: flex;
		align-items: baseline;
		gap: var(--space-sm);
		border-top: 1px solid var(--color-rule);
		padding: var(--space-xs) 0;
		font-size: var(--text-sm);
	}

	.speed-row-dot {
		align-self: center;
		flex-shrink: 0;
		width: 10px;
		height: 10px;
		border-radius: 999px;
		border-width: 2px;
		border-style: solid;
	}

	.speed-row-hollow {
		background: transparent !important;
	}

	.speed-row-rnx .speed-row-name {
		font-weight: 600;
		color: var(--color-signal);
	}

	.speed-row-name {
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		color: var(--color-ink);
		white-space: nowrap;
	}

	.speed-row-nums {
		margin-left: auto;
		text-align: right;
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		color: var(--color-muted);
	}

	.speed-row-fig {
		color: var(--color-ink);
	}

	.speed-facts {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 0 var(--space-lg);
		margin: var(--space-xl) 0 0;
		padding: 0;
	}

	.speed-fact {
		border-left: 1px solid var(--color-rule);
		padding-left: var(--space-md);
	}

	.speed-fact:first-child {
		border-left: 0;
		padding-left: 0;
	}

	.speed-fact dt {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.speed-fact dd {
		margin: var(--space-2xs) 0 0;
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		line-height: 1.6;
		color: var(--color-ink);
	}

	.speed-how {
		margin-top: var(--space-lg);
		font-size: var(--text-xs);
		line-height: 1.7;
		color: var(--color-muted);
	}

	.speed-how summary {
		cursor: pointer;
	}

	.speed-how summary:hover {
		color: var(--color-ink);
	}

	.speed-how p {
		margin-top: var(--space-xs);
		max-width: var(--measure);
	}

	.speed-how code {
		white-space: nowrap;
	}

	.speed-cmps {
		margin-top: var(--space-xs);
		list-style: none;
		padding: 0;
		font-family: var(--font-mono);
	}

	.speed-cmps li {
		display: grid;
		grid-template-columns: minmax(0, 9rem) 4.5rem minmax(0, 1fr) minmax(0, 1fr);
		gap: var(--space-sm);
		padding: 2px 0;
		color: rgb(var(--aura-text) / 0.55);
	}

	.speed-cmps li span:first-child {
		color: rgb(var(--aura-text) / 0.75);
	}

	.speed button:active {
		transform: translateY(1px);
	}

	@media (max-width: 760px) {
		.speed-facts {
			grid-template-columns: minmax(0, 1fr);
			gap: var(--space-sm) 0;
		}

		.speed-fact {
			border-left: 0;
			border-top: 1px solid var(--color-rule);
			padding-left: 0;
			padding-top: var(--space-xs);
		}

		.speed-fact:first-child {
			border-top: 0;
			padding-top: 0;
		}

		.speed-cmps li {
			grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
			gap: var(--space-3xs) var(--space-sm);
		}

		.speed-cmps li span:first-child {
			grid-column: 1 / -1;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.speed button:active {
			transform: none;
		}
	}
</style>
