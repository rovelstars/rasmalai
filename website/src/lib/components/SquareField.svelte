<script lang="ts">
	import { onMount } from 'svelte';

	// Slow-breathing square field behind the hero. A single canvas paints a
	// grid of small squares whose opacity oscillates on independent phases;
	// rows near the top are more likely to be alive than rows near the
	// bottom. Capped at ~10fps on a DPR-1 canvas (a few hundred fillRects
	// per frame), paused entirely under prefers-reduced-motion.
	let canvas = $state<HTMLCanvasElement | null>(null);

	interface Cell {
		x: number;
		y: number;
		vitality: number;
		phase: number;
		period: number;
		maxA: number;
		color: string;
		born: number;
		life: number;
		rest: number;
		deadUntil: number;
	}

	const CELL = 26;
	const GAP = 3;
	const SIZE = CELL - GAP;
	const FADE = 1400;

	function roll(now: number): Omit<Cell, 'x' | 'y' | 'vitality'> {
		const pick = Math.random();
		return {
			phase: Math.random() * Math.PI * 2,
			period: 5000 + Math.random() * 5000,
			maxA: 0.025 + Math.random() * 0.06,
			color: pick < 0.7 ? '162,119,255' : '130,226,255',
			born: now,
			life: 9000 + Math.random() * 9000,
			rest: 1500 + Math.random() * 5000,
			deadUntil: 0
		};
	}

	function seed(w: number, h: number, now: number): Cell[] {
		const cols = Math.ceil(w / CELL);
		const rows = Math.ceil(h / CELL);
		const cells: Cell[] = [];
		for (let r = 0; r < rows; r++) {
			const vitality = 0.48 - (r / Math.max(1, rows - 1)) * 0.43;
			for (let c = 0; c < cols; c++) {
				if (Math.random() > vitality) continue;
				cells.push({
					x: c * CELL + GAP / 2,
					y: r * CELL + GAP / 2,
					vitality,
					...roll(now - Math.random() * 12000)
				});
			}
		}
		return cells;
	}

	onMount(() => {
		const el = canvas;
		if (!el) return;
		const ctx = el.getContext('2d');
		if (!ctx) return;
		const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

		let cells: Cell[] = [];
		let raf = 0;
		let last = 0;

		function resize() {
			const rect = el!.getBoundingClientRect();
			el!.width = Math.max(1, Math.floor(rect.width));
			el!.height = Math.max(1, Math.floor(rect.height));
			cells = seed(el!.width, el!.height, performance.now());
		}

		function envelope(c: Cell, now: number): number {
			if (now < c.deadUntil) return 0;
			const age = now - c.born;
			if (age < 0) return 0;
			if (age < c.life) {
				const fin = Math.min(1, age / FADE);
				const fout = Math.min(1, (c.life - age) / FADE);
				return Math.min(fin, fout);
			}
			if (Math.random() < c.vitality) {
				Object.assign(c, roll(now));
				return 0;
			}
			c.deadUntil = now + (c.rest * (0.5 + Math.random())) / Math.max(0.05, c.vitality);
			return 0;
		}

		function draw(now: number) {
			ctx!.clearRect(0, 0, el!.width, el!.height);
			for (const c of cells) {
				const env = envelope(c, now);
				if (env <= 0) continue;
				const wave = 0.5 + 0.5 * Math.sin((now / c.period) * Math.PI * 2 + c.phase);
				const a = c.maxA * (0.35 + 0.65 * wave) * env;
				ctx!.fillStyle = `rgba(${c.color},${a.toFixed(3)})`;
				ctx!.fillRect(c.x, c.y, SIZE, SIZE);
			}
		}

		const ro = new ResizeObserver(resize);
		ro.observe(el);
		resize();

		if (reduced) {
			draw(1200);
			return () => ro.disconnect();
		}
		function tick(now: number) {
			if (now - last > 100) {
				last = now;
				draw(now);
			}
			raf = requestAnimationFrame(tick);
		}
		raf = requestAnimationFrame(tick);
		return () => {
			cancelAnimationFrame(raf);
			ro.disconnect();
		};
	});
</script>

<canvas
	bind:this={canvas}
	class="squarefield pointer-events-none absolute inset-x-0 top-0 h-[620px] w-full"
	aria-hidden="true"
></canvas>

<style>
	.squarefield {
		-webkit-mask-image: linear-gradient(to bottom, black 20%, transparent 92%);
		mask-image: linear-gradient(to bottom, black 20%, transparent 92%);
	}
</style>
