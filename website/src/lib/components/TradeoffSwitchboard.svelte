<script lang="ts">
	import AuraCode from '$lib/components/AuraCode.svelte';

	interface Option {
		id: string;
		index: string;
		label: string;
		taxTitle: string;
		tax: string;
		bypassTitle: string;
		bypass: string;
		code: string;
	}

	const OPTIONS: Option[] = [
		{
			id: 'cpp',
			index: '01',
			label: 'C / C++',
			taxTitle: 'The tax: build graphs and memory CVEs',
			tax: 'Four thousand lines of CMake to produce one binary, then dangling pointers and use-after-free bugs shipping as CVEs. The machine gives you everything and checks nothing.',
			bypassTitle: 'The bypass: one manifest, checked ownership',
			bypass:
				'A Project.config that fits on an index card, scopes that free what they own, and C libraries declared in four lines instead of a binding generator.',
			code: `import {
    fn compressBound(n: Int): Int
} from native "z"`
		},
		{
			id: 'go',
			index: '02',
			label: 'Go',
			taxTitle: 'The tax: pauses you cannot schedule',
			tax: 'Stop-the-world GC pauses landing in the middle of audio callbacks and game frames. Fast builds, then a runtime that stops your program to clean up after it.',
			bypassTitle: 'The bypass: scopes free on time, every time',
			bypass:
				'Values die when their block ends — deterministically, on the releasing thread. Cleanup reads like Go defer, because it is defer, without a collector behind it.',
			code: `fn handle(id: Int): Int {
    defer { print("done"); }
    return id * 2;
}`
		},
		{
			id: 'rust',
			index: '03',
			label: 'Rust',
			taxTitle: 'The tax: puzzles and waiting',
			tax: 'A thirty-minute borrow-checker negotiation over a circular graph, then a five-minute cold build to find out if you won. Real safety, billed hourly.',
			bypassTitle: 'The bypass: the closing brace is the free point',
			bypass:
				'Lexical scopes do the job lifetimes did — no annotations on linear logic, no refactors to satisfy the checker. This loop compiles in milliseconds and frees itself.',
			code: `fn total(xs: Array<Int>): Int {
    let sum = 0;
    for x in xs { sum = sum + x; }
    return sum;
}`
		},
		{
			id: 'js',
			index: '04',
			label: 'JavaScript / TS',
			taxTitle: 'The tax: surprises at runtime',
			tax: 'Dynamic types mean the bug arrives in production, the event loop colors every function async, and shipping means configuring a bundler to undo it all.',
			bypassTitle: 'The bypass: hardware types, straight-line code',
			bypass:
				'Int is 64 bits everywhere, mixing float kinds is a compile error instead of a silent precision change, and concurrency is real threads — no coloring.',
			code: `fn clamp(v: Int, lo: Int, hi: Int): Int {
    if v < lo { return lo; }
    if v > hi { return hi; }
    return v;
}`
		}
	];

	let sel = $state(OPTIONS[2]);
	let dir = $state(0);
	let first = $state(true);

	function pick(o: Option) {
		if (o.id === sel.id) return;
		const at = (id: string) => OPTIONS.findIndex((x) => x.id === id);
		dir = at(o.id) > at(sel.id) ? 1 : -1;
		first = false;
		sel = o;
	}
</script>

<div class="bay" data-testid="tradeoff-switchboard">
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-aura-border px-4 py-2 font-mono text-[11px]">
		<span class="text-aura-muted">switchboard - pick the tax you know</span>
		<span class="ml-auto hidden text-aura-muted sm:inline">[ CIRCUIT: 04 ]</span>
	</div>

	<div
		class="grid grid-cols-2 gap-1 border-b border-aura-border p-2 sm:grid-cols-4"
		role="group"
		aria-label="Language selector"
	>
		{#each OPTIONS as o}
			<button
				onclick={() => pick(o)}
				aria-pressed={sel.id === o.id}
				class="press border px-3 py-2 text-left font-mono text-[12px] {sel.id === o.id
					? 'border-aura-purple/50 bg-aura-purple/10 text-aura-purple'
					: 'border-aura-border text-aura-muted hover:border-aura-borderHover hover:text-aura-text'}"
			>
				<span class="mr-2 text-[10px] opacity-60">{o.index}</span>{o.label}
			</button>
		{/each}
	</div>

	{#key sel.id}
		<div class="grid overflow-hidden lg:grid-cols-2 {first ? '' : dir < 0 ? 'sw-dir-l' : 'sw-dir-r'}">
			<div class="border-b border-aura-border lg:border-r lg:border-b-0">
				<p class="border-b border-aura-border px-4 py-1.5 font-mono text-[11px] text-aura-red">
					+ chamber 1 - the tax you pay
				</p>
				<div class="border-l-2 border-aura-red px-4 py-4 md:px-6">
					<h3 class="font-bold text-aura-text">{sel.taxTitle}</h3>
					<p class="mt-2 max-w-[60ch] text-sm leading-relaxed text-aura-text/70">{sel.tax}</p>
				</div>
			</div>
			<div>
				<p class="border-b border-aura-border px-4 py-1.5 font-mono text-[11px] text-aura-green">
					+ chamber 2 - the rasmalai bypass
				</p>
				<div class="border-l-2 border-aura-green px-4 py-4 md:px-6">
					<h3 class="font-bold text-aura-text">{sel.bypassTitle}</h3>
					<p class="mt-2 max-w-[60ch] text-sm leading-relaxed text-aura-text/70">{sel.bypass}</p>
					<div class="mt-3 overflow-hidden border border-aura-border">
						<AuraCode source={sel.code} />
					</div>
				</div>
			</div>
		</div>
	{/key}
</div>
