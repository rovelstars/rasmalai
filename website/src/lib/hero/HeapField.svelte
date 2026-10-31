<script lang="ts">
	import { onMount } from 'svelte';
	import { Play, Pause, SkipBack, SkipForward } from 'lucide-svelte';
	import { highlightAura } from '$lib/aura/highlight';

	const CW = 8;
	const CH = 16;
	const FONT = 13;
	const MONO = '"JetBrains Mono", ui-monospace, monospace';

	interface Scene {
		title: string;
		caption: string;
		lir: string;
		first: number;
		source: string[];
		duration: number;
	}

	const SCENES: Scene[] = [
		{
			title: '01 / one value, two names',
			caption:
				'Two names, one array. The count rises on the copy, falls on the reassignment, and the last release frees the array on this thread.',
			lir: 'obj_new %0   retain %0   obj_new %1   release %0   deinit %0',
			first: 1,
			duration: 20,
			source: [
				'fn count(): Int {',
				'  let arr = [1, 2, 3]',
				'  let same = arr',
				'  arr = [4, 5]',
				'  let n = same.length()',
				'  return n',
				'}'
			]
		},
		{
			title: '02 / a handle that cannot dangle',
			caption:
				'A GenRef keeps a slot and an epoch, not a pointer. When the owner dies the epoch moves, so the handle reads null instead of freed memory.',
			lir: 'genref_of %0   genref_get %1   genref_invalidate %0',
			first: 2,
			duration: 20,
			source: [
				'  let weak: GenRef<Node>? = null',
				'  do {',
				'    let temp = new Node(99)',
				'    weak = GenRef(temp)',
				'  } while false',
				'  guard let stale = weak.get() else { return 42 }',
				'  return stale.value'
			]
		}
	];

	const STEPS: number[][] = [
		[1, 2, 3, 4, 5, 6],
		[0, 1, 2, 3, 4, 5, 6]
	];

	const CUTS: number[][] = [
		[0.22, 0.44, 0.66, 0.82, 0.92, 0.99],
		[0.18, 0.3, 0.4, 0.48, 0.62, 0.82, 0.99]
	];

	let canvas: HTMLCanvasElement;
	let host: HTMLDivElement;

	function escHtml(s: string): string {
		return s
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;');
	}

	function richLine(text: string): string {
		return highlightAura(text)
			.map((s) => `<span class="${s.cls}">${escHtml(s.text)}</span>`)
			.join('');
	}

	let ctx: CanvasRenderingContext2D | null = null;
	let width = 0;
	let height = 0;
	let dpr = 1;
	let frame = 0;
	let playing = $state(true);
	let visible = true;
	let reduced = false;
	let lastFrame = 0;
	let clock = 7.2;
	let scene = $state(0);
	let line = $state(1);
	let gauge = $state('0 freed - 0 pauses');

	let rich = $derived(SCENES[scene].source.map(richLine));

	let resizeObserver: ResizeObserver | undefined;
	let themeObserver: MutationObserver | undefined;
	let intersectionObserver: IntersectionObserver | undefined;

	interface Palette {
		ink: string;
		ink2: string;
		muted: string;
		faint: string;
		rule: string;
		purple: string;
		green: string;
		cyan: string;
		orange: string;
		red: string;
	}

	let palette: Palette;

	function readPalette(): Palette {
		const style = getComputedStyle(document.documentElement);
		const tone = (name: string, alpha = 1) => {
			const parts = style.getPropertyValue(name).trim().split(/\s+/).map(Number);
			return `rgba(${parts[0]}, ${parts[1]}, ${parts[2]}, ${alpha})`;
		};
		return {
			ink: tone('--aura-text'),
			ink2: tone('--aura-text', 0.82),
			muted: tone('--aura-muted'),
			faint: tone('--aura-faint'),
			rule: tone('--aura-lineStrong'),
			purple: tone('--aura-purple'),
			green: tone('--aura-green'),
			cyan: tone('--aura-cyan'),
			orange: tone('--aura-orange'),
			red: tone('--aura-red')
		};
	}

	interface Geo {
		cols: number;
		rows: number;
		panelX: number;
		panelW: number;
		panelTop: number;
		panelBottom: number;
		localsTop: number;
		localsRows: number;
		heapTop: number;
		heapBottom: number;
		slots: number;
		slotH: number;
		railY: number;
		tickY: number;
		wide: boolean;
		split: boolean;
		leftW: number;
		divX: number;
		logX: number;
		logW: number;
	}

	let geo: Geo;

	function measure(): Geo {
		const cols = Math.max(26, Math.floor(width / CW));
		const rows = Math.max(14, Math.floor(height / CH));
		const gutter = 2;
		const panelX = gutter;
		const panelW = cols - gutter * 2;
		const panelTop = 2;
		const panelBottom = rows - 4;
		const localsTop = panelTop + 2;
		const localsRows = 2;
		const heapTop = localsTop + localsRows + 2;
		const heapBottom = panelBottom - 1;
		const room = Math.max(6, heapBottom - heapTop);
		let slots = Math.max(2, Math.min(4, Math.floor(room / 4)));
		let slotH = Math.floor(room / slots);
		if (slotH < 3) {
			slotH = 3;
			slots = Math.max(1, Math.floor(room / slotH));
		}
		const split = panelW >= 58;
		const leftW = split ? Math.round(panelW * 0.64) : panelW;
		return {
			cols,
			rows,
			panelX,
			panelW,
			panelTop,
			panelBottom,
			localsTop,
			localsRows,
			heapTop,
			heapBottom,
			slots,
			slotH,
			railY: panelBottom + 2,
			tickY: panelBottom + 3,
			wide: width >= 520,
			split,
			leftW,
			divX: panelX + leftW,
			logX: panelX + leftW + 2,
			logW: panelW - leftW - 3
		};
	}

	function slotRow(index: number): number {
		return geo.heapTop + index * geo.slotH;
	}

	function put(c: number, r: number, ch: string, color: string, alpha: number) {
		if (!ctx || alpha <= 0.012 || c < 0 || r < 0 || c >= geo.cols || r >= geo.rows) return;
		ctx.globalAlpha = Math.min(1, alpha);
		ctx.fillStyle = color;
		ctx.fillText(ch, c * CW, r * CH + CH / 2 + 1);
	}

	type CellPut = (c: number, r: number, ch: string, color: string, alpha: number) => void;

	function txt(c: number, r: number, s: string, color: string, alpha: number, over?: CellPut) {
		for (let i = 0; i < s.length; i++) {
			if (over) over(c + i, r, s[i], color, alpha);
			else put(c + i, r, s[i], color, alpha);
		}
	}

	function txtRight(end: number, r: number, s: string, color: string, alpha: number, over?: CellPut) {
		txt(end - s.length, r, s, color, alpha, over);
	}

	function rule(c: number, r: number, n: number, color: string, alpha: number, ch = '-') {
		for (let i = 0; i < n; i++) put(c + i, r, ch, color, alpha);
	}

	function clamp01(v: number) {
		return Math.max(0, Math.min(1, v));
	}

	function ramp(u: number, from: number, to: number) {
		return clamp01((u - from) / Math.max(0.0001, to - from));
	}

	function ease(t: number) {
		return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
	}

	function drawFrame() {
		if (!ctx) return;
		const g = geo;
		const x = g.panelX;
		const w = g.panelW;
		const y0 = g.panelTop;
		const y1 = g.panelBottom;

		put(x, y0, '+', palette.rule, 0.72);
		put(x + w - 1, y0, '+', palette.rule, 0.72);
		put(x, y1, '+', palette.rule, 0.72);
		put(x + w - 1, y1, '+', palette.rule, 0.72);
		rule(x + 1, y0, w - 2, palette.rule, 0.38);
		rule(x + 1, y1, w - 2, palette.rule, 0.38);
		for (let r = y0 + 1; r < y1; r++) {
			put(x, r, '|', palette.rule, 0.3);
			put(x + w - 1, r, '|', palette.rule, 0.24);
		}

		const sep2 = g.heapTop - 1;
		rule(x + 1, sep2, g.leftW - 2, palette.rule, 0.3);
		put(x + g.leftW - 1, sep2, '+', palette.rule, 0.5);

		for (let i = 1; i < g.slots; i++) {
			const r = slotRow(i) - 1;
			rule(x + 1, r, g.leftW - 2, palette.rule, 0.22);
			put(x + g.leftW - 1, r, '+', palette.rule, 0.45);
		}

		for (let r = g.heapTop; r < g.panelBottom; r++) {
			for (let c = x + 2; c < x + g.leftW - 2; c += 2) put(c, r, '.', palette.faint, 0.11);
		}

		if (g.split) {
			const dx = g.divX;
			for (let r = g.panelTop + 1; r < g.panelBottom; r++) put(dx, r, '|', palette.rule, 0.3);
		}
	}

	function heading(row: number, label: string, note: string) {
		const g = geo;
		txt(g.panelX + 2, row, label, palette.faint, 0.72);
		if (note) txtRight(g.panelX + g.leftW - 3, row, note, palette.faint, 0.52);
	}

	interface Entry {
		at: number;
		op: string;
		addr: string;
		note: string;
		tone: string;
	}

	function drawLog(entries: Entry[], u: number) {
		const g = geo;
		if (!g.split) return;
		txt(g.logX, g.panelTop + 1, 'events', palette.faint, 0.62);
		const live = entries.filter((e) => u >= e.at);
		txtRight(g.panelX + g.panelW - 3, g.panelTop + 1, String(live.length), palette.faint, 0.36);
		const rows = g.panelBottom - g.panelTop - 3;
		const shown = live.slice(-rows);
		const wide = g.logW >= 27;
		shown.forEach((e, i) => {
			const y = g.panelTop + 2 + i;
			const fresh = clamp01((u - e.at) / 0.03);
			txt(g.logX, y, e.op, e.tone, 0.85 * fresh);
			if (wide) {
				txt(g.logX + 10, y, e.addr, palette.ink2, 0.7 * fresh);
				txt(g.logX + 18, y, e.note, palette.faint, 0.7 * fresh);
			} else {
				txt(g.logX + 9, y, e.note, palette.ink2, 0.62 * fresh);
			}
		});
	}

	interface Binding {
		name: string;
		target: string;
		type: string;
		alpha: number;
		struck: number;
		flash?: number;
		dotted?: boolean;
	}

	function drawBindings(list: Binding[]) {
		const g = geo;
		const colName = g.panelX + 2;
		const colArrow = colName + 8;
		const colTarget = colArrow + 5;
		const colType = colTarget + 13;

		list.forEach((b, i) => {
			if (b.alpha <= 0.02) return;
			const y = g.localsTop + i;
			const dead = b.struck > 0;
			txt(colName, y, b.name, dead ? palette.faint : palette.ink2, b.alpha * (dead ? 0.45 : 0.92));
			if (dead) rule(colName, y, b.name.length, palette.red, b.alpha * b.struck * 0.5);
			const tone = b.dotted ? palette.cyan : palette.purple;
			const glyph = b.dotted ? ':' : '-';
			for (let c = colArrow; c < colArrow + 3; c++) {
				put(c, y, glyph, tone, b.alpha * (dead ? 0.14 : 0.55));
			}
			put(colArrow + 3, y, '>', tone, b.alpha * (dead ? 0.14 : 0.85));
			const valueAlpha = b.alpha * (dead ? 0.3 : 0.95);
			const flash = b.flash ?? 0;
			if (flash > 0) {
				txt(colTarget, y, b.target, tone, valueAlpha * (1 - flash));
				txt(colTarget, y, b.target, palette.ink, valueAlpha * flash);
			} else {
				txt(colTarget, y, b.target, tone, valueAlpha);
			}
			if (g.wide) txt(colType, y, b.type, palette.faint, b.alpha * (dead ? 0.25 : 0.7));
		});
	}

	interface Block {
		addr: string;
		type: string;
		detail: string;
		meta: string;
		owners: string;
		size: string;
		rc: number | null;
		alpha: number;
		dissolve: number;
		hot: number;
	}

	function drawBlock(index: number, b: Block | null, addr: string, freed = '') {
		const g = geo;
		const y = slotRow(index);
		const x0 = g.panelX + 2;
		const right = g.panelX + g.leftW - 3;
		const left = x0 + 8;

		if (!b) {
			txt(x0, y, addr, palette.faint, 0.5);
			txt(left, y, 'free', palette.faint, 0.46);
			if (freed) txtRight(right, y, freed, palette.faint, 0.4);
			for (let r = y + 1; r < y + g.slotH; r++) {
				for (let c = x0; c < right; c += 2) put(c, r, '.', palette.faint, 0.24);
			}
			return;
		}

		const d = b.dissolve;
		const wipe = d > 0 ? x0 + (right - x0) - Math.ceil((right - x0) * ease(d)) : 1e9;
		const over: CellPut = (c, r, ch, colour, alpha) => {
			if (c >= wipe) {
				if (d > 0) put(c, r, (c + r) % 5 === 0 ? ':' : '.', palette.faint, 0.22 * (1 - d));
				return;
			}
			put(c, r, ch, colour, alpha);
		};

		const dim = b.alpha * (d > 0 ? 1 - d * 0.7 : 1);

		txt(x0, y, b.addr, palette.faint, dim * 0.8, over);
		txt(left, y, b.type, palette.ink2, dim * 0.95, over);
		txtRight(right - 2, y, 'rc', palette.faint, dim * 0.6, over);
		txtRight(right, y, String(b.rc ?? 0), b.rc === 0 ? palette.red : palette.purple, dim, over);
		txt(left, y + 1, b.detail, palette.ink, dim, over);
		if (b.size) txtRight(right, y + 1, b.size, palette.faint, dim * 0.6, over);
		if (d > 0.3) {
			txt(left, y + 2, 'deinit - bytes returned', palette.green, clamp01((d - 0.3) * 2.4), over);
		} else {
			if (g.slotH >= 4) txt(left, y + 2, b.meta, palette.faint, dim * 0.75, over);
			if (g.slotH >= 5 && b.owners) txt(left, y + 3, b.owners, palette.muted, dim * 0.6, over);
		}
		if (b.hot > 0) {
			for (let c = x0; c < right; c++) put(c, y, '-', palette.purple, b.hot * 0.16);
		}
	}

	function pulse(row: number, from: number, to: number, t: number, glyph: string, color: string) {
		if (t <= 0 || t >= 1) return;
		const lo = Math.min(from, to);
		const hi = Math.max(from, to) - glyph.length + 1;
		const c = Math.max(lo, Math.min(hi, Math.round(from + (to - from) * ease(t))));
		txt(c, row, glyph, color, Math.sin(Math.PI * t) * 0.95);
	}

	function drawRails(s: Scene) {
		const g = geo;
		if (!g.wide) {
			if (g.railY < g.rows) txt(g.panelX + 2, g.railY, '4 backends - one output', palette.cyan, 0.5);
			if (g.tickY < g.rows) txt(g.panelX + 2, g.tickY, 'no collector - 0 pauses', palette.faint, 0.45);
			return;
		}
		if (g.railY >= g.rows) return;
		txt(g.panelX + 2, g.railY, 'lir', palette.faint, 0.5);
		let c = g.panelX + 6;
		const width = g.panelX + g.panelW - 2;
		for (const op of s.lir.split('   ')) {
			if (c + op.length >= width) break;
			txt(c, g.railY, op, palette.cyan, 0.44);
			c += op.length + 3;
		}
		if (g.tickY >= g.rows) return;
		let x = g.panelX + 2;
		txt(x, g.tickY, 'same program', palette.faint, 0.42);
		x += 14;
		for (const label of ['interp', 'cranelift', 'llvm', 'aot']) {
			if (x + label.length + 4 >= width) break;
			txt(x, g.tickY, label, palette.muted, 0.5);
			txt(x + label.length + 1, g.tickY, 'ok', palette.green, 0.62);
			x += label.length + 6;
		}
	}

	function drawSceneOne(u: number) {
		const g = geo;
		const tAlloc = 0.01;
		const tShare = 0.24;
		const tSwap = 0.45;
		const tExit = 0.68;

		const dissolveA = ramp(u, tExit + 0.03, tExit + 0.15);
		const dissolveB = ramp(u, tExit + 0.1, tExit + 0.22);
		const rcA = u >= tExit ? 0 : u >= tSwap ? 1 : u >= tShare ? 2 : 1;

		const aIn = ramp(u, tAlloc, tAlloc + 0.05);
		const bIn = ramp(u, tSwap, tSwap + 0.05);
		const rcB = u >= tExit ? 0 : 1;

		const gone = ramp(u, tExit, tExit + 0.07);
		const bindingCount = (u >= tAlloc && gone < 0.5 ? 1 : 0) + (u >= tShare && gone < 0.5 ? 1 : 0);
		heading(g.panelTop + 1, 'locals', `${bindingCount} ${bindingCount === 1 ? 'binding' : 'bindings'}`);
		drawBindings([
			{
				name: 'arr',
				target: u >= tSwap ? '0x0018' : '0x0000',
				type: 'Array<Int>',
				alpha: ramp(u, tAlloc, tAlloc + 0.06),
				struck: ramp(u, tExit, tExit + 0.06),
				flash: u >= tSwap ? 1 - ramp(u, tSwap, tSwap + 0.05) : 0
			},
			{
				name: 'same',
				target: '0x0000',
				type: 'same object',
				alpha: ramp(u, tShare, tShare + 0.06),
				struck: ramp(u, tExit, tExit + 0.06)
			}
		]);

		let live = 0;
		let bytes = 0;
		if (aIn > 0.02 && dissolveA < 0.5) {
			live += 1;
			bytes += 24;
		}
		if (bIn > 0.02 && dissolveB < 0.5) {
			live += 1;
			bytes += 16;
		}
		heading(g.heapTop - 2, 'heap', `${live} live - ${bytes} B`);

		const hotA = Math.max(
			(1 - Math.abs(u - tShare) / 0.03) * ramp(u, tShare - 0.02, tShare),
			(1 - Math.abs(u - tSwap) / 0.03) * ramp(u, tSwap - 0.02, tSwap),
			(1 - Math.abs(u - tExit) / 0.03) * ramp(u, tExit - 0.02, tExit)
		);

		if (aIn > 0.02 && dissolveA < 0.97) {
			drawBlock(
				0,
				{
					addr: '0x0000',
					type: 'Array<Int>',
					detail: '[ 1, 2, 3 ]',
					meta: 'gen 1 - id 0x03 - ARC',
					owners: 'owners  arr, same',
					size: '24 B',
					rc: rcA,
					alpha: aIn,
					dissolve: dissolveA,
					hot: Math.max(0, hotA)
				},
				'0x0000'
			);
		} else {
			drawBlock(0, null, '0x0000', '24 B');
		}

		if (bIn > 0.02 && dissolveB < 0.97) {
			drawBlock(
				1,
				{
					addr: '0x0018',
					type: 'Array<Int>',
					detail: '[ 4, 5 ]',
					meta: 'gen 1 - id 0x07 - ARC',
					owners: 'owners  arr',
					size: '16 B',
					rc: rcB,
					alpha: bIn,
					dissolve: dissolveB,
					hot: Math.max(0, (1 - Math.abs(u - tSwap) / 0.03) * ramp(u, tSwap - 0.02, tSwap))
				},
				'0x0018'
			);
		} else {
			drawBlock(1, null, '0x0018', '16 B');
		}

		for (let i = 2; i < g.slots; i++) {
			drawBlock(i, null, '0x' + (0x28 + (i - 2) * 0x20).toString(16).padStart(4, '0'));
		}

		drawLog(
			[
				{ at: tAlloc, op: 'alloc', addr: '0x0000', note: '24 B', tone: palette.purple },
				{ at: tAlloc + 0.01, op: 'retain', addr: '0x0000', note: 'rc 0->1', tone: palette.green },
				{ at: tShare, op: 'retain', addr: '0x0000', note: 'rc 1->2', tone: palette.green },
				{ at: tSwap, op: 'alloc', addr: '0x0018', note: '16 B', tone: palette.purple },
				{ at: tSwap + 0.01, op: 'release', addr: '0x0000', note: 'rc 2->1', tone: palette.orange },
				{ at: tExit, op: 'release', addr: '0x0000', note: 'rc 1->0', tone: palette.orange },
				{ at: tExit + 0.02, op: 'deinit', addr: '0x0000', note: '24 B', tone: palette.red },
				{ at: tExit + 0.05, op: 'release', addr: '0x0018', note: 'rc 1->0', tone: palette.orange },
				{ at: tExit + 0.07, op: 'deinit', addr: '0x0018', note: '16 B', tone: palette.red }
			],
			u
		);

		const colArrow = g.panelX + 2 + 8;
		pulse(g.localsTop, colArrow + 3, colArrow, ramp(u, tAlloc, tAlloc + 0.1), '+1', palette.purple);
		pulse(g.localsTop, colArrow + 3, colArrow, ramp(u, tShare, tShare + 0.1), '+1', palette.purple);
		pulse(g.localsTop, colArrow, colArrow + 3, ramp(u, tSwap, tSwap + 0.1), '-1', palette.orange);
		pulse(g.localsTop + 1, colArrow + 3, colArrow, ramp(u, tShare, tShare + 0.1), '+1', palette.purple);
		pulse(g.localsTop + 1, colArrow, colArrow + 3, ramp(u, tExit, tExit + 0.1), '-1', palette.orange);
	}

	function drawSceneTwo(u: number) {
		const g = geo;
		const tAlloc = 0.01;
		const tHandle = 0.16;
		const tDrop = 0.42;
		const tRead = 0.62;

		const drop = ramp(u, tDrop, tDrop + 0.14);
		const stale = u >= tDrop + 0.02;

		const sceneBindings = (u >= tAlloc ? 1 : 0) + (u >= tHandle ? 1 : 0);
		heading(
			g.panelTop + 1,
			'locals',
			`${sceneBindings} ${sceneBindings === 1 ? 'binding' : 'bindings'}`
		);
		drawBindings([
			{
				name: 'temp',
				target: '0x0040',
				type: 'Node',
				alpha: ramp(u, tAlloc, tAlloc + 0.06),
				struck: ramp(u, tDrop, tDrop + 0.06)
			},
			{
				name: 'weak',
				target: stale ? 'slot 3 - e8' : 'slot 3 - e7',
				type: stale ? 'stale' : 'GenRef<Node>',
				alpha: ramp(u, tHandle, tHandle + 0.06),
				struck: 0,
				flash: stale ? 1 - ramp(u, tDrop, tDrop + 0.05) : 0,
				dotted: true
			}
		]);

		heading(g.heapTop - 2, 'heap', u >= tAlloc && drop < 0.5 ? '1 live - 16 B' : '0 live - 0 B');

		drawBlock(
			0,
			{
				addr: '0x0040',
				type: 'Node',
				detail: 'value 99',
				meta: 'gen 1 - id 0x0b - ARC',
				owners: 'owners  temp',
				size: '16 B',
				rc: u >= tDrop ? 0 : 1,
				alpha: ramp(u, tAlloc, tAlloc + 0.05),
				dissolve: drop,
				hot: Math.max(0, (1 - Math.abs(u - tDrop) / 0.03) * ramp(u, tDrop - 0.02, tDrop))
			},
			'0x0040'
		);

		for (let i = 1; i < g.slots; i++) {
			drawBlock(i, null, '0x' + (0x50 + (i - 1) * 0x20).toString(16).padStart(4, '0'));
		}

		drawLog(
			[
				{ at: tAlloc, op: 'alloc', addr: '0x0040', note: '16 B', tone: palette.purple },
				{ at: tAlloc + 0.01, op: 'retain', addr: '0x0040', note: 'rc 0->1', tone: palette.green },
				{ at: tHandle, op: 'genref', addr: 'slot 3', note: 'e7', tone: palette.cyan },
				{ at: tDrop, op: 'release', addr: '0x0040', note: 'rc 1->0', tone: palette.orange },
				{ at: tDrop + 0.02, op: 'deinit', addr: '0x0040', note: '16 B', tone: palette.red },
				{ at: tDrop + 0.04, op: 'invalid', addr: 'slot 3', note: 'e7->e8', tone: palette.cyan },
				{ at: tRead, op: 'get', addr: 'slot 3', note: 'null', tone: palette.red },
				{ at: tRead + 0.06, op: 'guard', addr: '', note: 'return 42', tone: palette.green }
			],
			u
		);

		const colArrow = g.panelX + 2 + 8;
		pulse(g.localsTop + 1, colArrow, colArrow + 3, ramp(u, tHandle, tHandle + 0.1), 'e7', palette.cyan);
		pulse(g.localsTop + 1, colArrow + 3, colArrow, ramp(u, tRead, tRead + 0.12), 'null', palette.red);
	}

	function lineAt(index: number, u: number) {
		const cuts = CUTS[index];
		const steps = STEPS[index];
		for (let i = 0; i < cuts.length; i++) {
			if (u < cuts[i]) return steps[Math.min(i, steps.length - 1)];
		}
		return steps[steps.length - 1];
	}

	function locate(t: number) {
		const total = SCENES.reduce((sum, s) => sum + s.duration, 0);
		let rest = ((t % total) + total) % total;
		let index = 0;
		while (rest >= SCENES[index].duration) {
			rest -= SCENES[index].duration;
			index += 1;
		}
		return { index, u: rest / SCENES[index].duration };
	}

	function draw() {
		if (!ctx || width === 0 || height === 0) return;
		ctx.clearRect(0, 0, width, height);
		ctx.font = `500 ${FONT}px ${MONO}`;
		ctx.textAlign = 'left';
		ctx.textBaseline = 'middle';
		ctx.globalAlpha = 1;

		const { index, u } = locate(clock);
		drawFrame();
		if (index === 0) drawSceneOne(u);
		else drawSceneTwo(u);
		drawRails(SCENES[index]);
		ctx.globalAlpha = 1;
	}

	function onResize() {
		if (!canvas || !host) return;
		const rect = host.getBoundingClientRect();
		dpr = Math.min(window.devicePixelRatio || 1, 2);
		width = rect.width;
		height = rect.height;
		canvas.width = Math.round(width * dpr);
		canvas.height = Math.round(height * dpr);
		ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
		geo = measure();
		draw();
	}

	function tick(now: number) {
		frame = 0;
		if (!playing || !visible) return;
		if (now - lastFrame < 32) {
			frame = requestAnimationFrame(tick);
			return;
		}
		const dt = Math.min(0.12, (now - lastFrame) / 1000);
		lastFrame = now;
		clock += dt;
		const at = locate(clock);
		if (at.index !== scene) scene = at.index;
		const next = lineAt(at.index, at.u);
		if (next !== line) line = next;
		const freed = at.index === 0 ? (at.u > 0.86 ? 2 : at.u > 0.74 ? 1 : 0) : at.u > 0.56 ? 1 : 0;
		const text = `${freed} freed - 0 pauses`;
		if (text !== gauge) gauge = text;
		draw();
		frame = requestAnimationFrame(tick);
	}

	function schedule() {
		if (frame || !playing || !visible) return;
		frame = requestAnimationFrame(tick);
	}

	function stop() {
		if (frame) cancelAnimationFrame(frame);
		frame = 0;
	}

	function toggle() {
		playing = !playing;
		if (playing) {
			lastFrame = 0;
			schedule();
		} else {
			stop();
		}
		draw();
	}

	function sceneStart(index: number): number {
		let t = 0;
		for (let i = 0; i < index; i++) t += SCENES[i].duration;
		return t;
	}

	function jumpTo(t: number) {
		const total = SCENES.reduce((s, x) => s + x.duration, 0);
		clock = ((t % total) + total) % total;
		const at = locate(clock);
		if (at.index !== scene) scene = at.index;
		line = lineAt(at.index, at.u);
		lastFrame = 0;
		draw();
	}

	function prevScene() {
		const start = sceneStart(scene);
		jumpTo(clock - start > 2 ? start : sceneStart((scene + SCENES.length - 1) % SCENES.length));
	}

	function nextScene() {
		jumpTo(sceneStart((scene + 1) % SCENES.length));
	}

	function onVisibility() {
		visible = document.visibilityState === 'visible';
		if (visible && playing) schedule();
		else stop();
	}

	onMount(() => {
		ctx = canvas.getContext('2d');
		palette = readPalette();
		reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		resizeObserver = new ResizeObserver(onResize);
		resizeObserver.observe(host);
		themeObserver = new MutationObserver(() => {
			palette = readPalette();
			draw();
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class']
		});
		document.fonts.ready.then(onResize).catch(() => onResize());
		onResize();
		playing = !reduced;
		if (reduced) clock = 7.2;
		const at = locate(clock);
		scene = at.index;
		line = lineAt(at.index, at.u);
		gauge = '0 freed - 0 pauses';
		intersectionObserver = new IntersectionObserver(
			(entries) => {
				visible = entries[0]?.isIntersecting ?? true;
				if (visible && playing) schedule();
				else stop();
			},
			{ threshold: 0.05 }
		);
		intersectionObserver.observe(host);
		document.addEventListener('visibilitychange', onVisibility);
		if (playing) schedule();
		else draw();
		return () => {
			stop();
			resizeObserver?.disconnect();
			themeObserver?.disconnect();
			intersectionObserver?.disconnect();
			document.removeEventListener('visibilitychange', onVisibility);
		};
	});
</script>

<figure class="field" aria-labelledby="field-title">
	<div class="field-head">
		<p class="field-title" id="field-title">{SCENES[scene].title}</p>
		<div class="field-tools">
			<p class="field-gauge" aria-hidden="true">{gauge}</p>
			<button
				class="field-tool"
				onclick={prevScene}
				aria-label="Previous act"
			>
				<SkipBack size={13} />
			</button>
			<button
				class="field-tool"
				onclick={toggle}
				aria-label={playing ? 'Pause the memory trace' : 'Play the memory trace'}
			>
				{#if playing}<Pause size={13} />{:else}<Play size={13} />{/if}
			</button>
			<button
				class="field-tool"
				onclick={nextScene}
				aria-label="Next act"
			>
				<SkipForward size={13} />
			</button>
		</div>
	</div>

	<ol class="field-code" aria-label="The program this act traces">
		{#each SCENES[scene].source as text, i}
			<li class:is-active={i === line}>
				<span class="field-num">{SCENES[scene].first + i}</span>
				<code>{@html rich[i]}</code>
			</li>
		{/each}
	</ol>

	<div class="field-box" bind:this={host}>
		<canvas bind:this={canvas} aria-hidden="true"></canvas>
	</div>

	<figcaption class="field-caption">
		<span>{SCENES[scene].caption}</span>
		<span class="field-key" aria-hidden="true">
			<span><b>rc</b> refcount</span>
			<span><b>gen</b> generation</span>
			<span><b>-></b> reference</span>
			<span><b>~</b> GenRef</span>
		</span>
	</figcaption>
</figure>

<style>
	.field {
		display: grid;
		grid-template-rows: auto auto minmax(0, 1fr) auto;
		gap: var(--space-md);
		margin: 0;
		min-width: 0;
		height: 100%;
	}

	.field-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--space-md);
		border-bottom: 1px solid var(--color-rule);
		padding-bottom: var(--space-sm);
	}

	.field-title {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--color-muted);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		min-width: 0;
	}

	.field-code {
		display: grid;
		margin: 0;
		padding: 0;
		list-style: none;
		font-family: var(--font-mono);
		font-size: 11px;
		line-height: 1.6;
		border-left: 1px solid var(--color-rule);
		/* Both acts run 7 lines: fix the height so swapping scenes never
		moves the instrument or the caption below it. */
		height: calc(7 * 1.6em);
		overflow: hidden;
	}

	.field-code li {
		display: flex;
		gap: var(--space-sm);
		padding: 0 var(--space-xs) 0 var(--space-sm);
		color: var(--color-faint);
		white-space: pre;
		overflow: hidden;
		transition:
			color 340ms ease,
			background-color 340ms ease,
			box-shadow 340ms ease;
	}

	.field-code li.is-active {
		color: var(--color-ink);
		background: rgb(var(--aura-text) / 0.05);
		box-shadow: inset 2px 0 0 var(--color-accent);
	}

	.field-code code {
		font-family: var(--font-mono);
		background: none;
		border: 0;
		padding: 0;
		color: inherit;
		opacity: 0.42;
		transition: opacity 340ms ease;
	}

	.field-code li.is-active code {
		opacity: 1;
	}

	.field-num {
		width: 2ch;
		text-align: right;
		color: var(--color-faint);
		font-variant-numeric: tabular-nums;
	}

	.field-tools {
		display: flex;
		align-items: center;
		gap: var(--space-sm);
		flex-shrink: 0;
	}

	.field-gauge {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
	}

	.field-tool {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		flex-shrink: 0;
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		color: var(--color-muted);
		background: var(--color-paper-2);
	}

	.field-tool:hover {
		border-color: var(--color-rule-strong);
		color: var(--color-ink);
	}

	.field-tool:focus-visible {
		outline: 2px solid var(--color-accent);
		outline-offset: 2px;
	}

	.field-box {
		position: relative;
		min-height: 400px;
	}

	canvas {
		display: block;
		width: 100%;
		height: 100%;
	}

	.field-caption {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: var(--space-2xs) var(--space-lg);
		font-size: var(--text-xs);
		line-height: 1.55;
		color: var(--color-muted);
		/* Longest caption measures 80px at desktop; 7em reserves it plus
		wrap headroom on narrow screens so the page below never moves. */
		min-height: 7em;
	}

	.field-caption > span:first-child {
		max-width: 54ch;
	}

	.field-key {
		display: flex;
		flex-wrap: wrap;
		gap: var(--space-2xs) var(--space-md);
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--color-faint);
	}

	.field-key b {
		color: var(--color-ink-2);
		font-weight: 500;
	}

	@media (max-width: 640px) {
		.field-gauge {
			display: none;
		}

		.field-code {
			font-size: 10px;
		}

		.field-box {
			min-height: 300px;
		}

		.field-caption {
			font-size: 11px;
		}
	}
</style>
