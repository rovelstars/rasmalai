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
				'Values die when their block ends. The release runs deterministically on the releasing thread. Cleanup reads like Go defer, because it is defer, without a collector behind it.',
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
				'Lexical scopes do the job lifetimes did: no annotations on linear logic, no refactors to satisfy the checker. This loop compiles in milliseconds and frees itself.',
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
				'Int is 64 bits everywhere, mixing float kinds is a compile error instead of a silent precision change, and concurrency uses real threads with no coloring.',
			code: `fn clamp(v: Int, lo: Int, hi: Int): Int {
    if v < lo { return lo; }
    if v > hi { return hi; }
    return v;
}`
		}
	];

	let sel = $state(OPTIONS[2]);

	function pick(o: Option) {
		if (o.id === sel.id) return;
		sel = o;
	}

	const strip = (s: string) => s.replace(/^The (tax|bypass):\s*/, '');
</script>

<div class="switchboard" data-testid="tradeoff-switchboard">
	<div class="sw-tabs" role="group" aria-label="Language selector">
		{#each OPTIONS as o}
			<button
				onclick={() => pick(o)}
				aria-pressed={sel.id === o.id}
				class="sw-tab"
				class:sw-tab-active={sel.id === o.id}
			>
				<span class="sw-index">{o.index}</span>
				{o.label}
			</button>
		{/each}
	</div>

	{#key sel.id}
		<div class="sw-body">
			<div class="sw-col">
				<h3 class="sw-title">
					<span class="sw-label sw-label-tax">The tax:</span>
					<span class="sw-frag">{strip(sel.taxTitle)}</span>
				</h3>
				<p class="sw-copy">{sel.tax}</p>
			</div>
			<div class="sw-col">
				<h3 class="sw-title">
					<span class="sw-label sw-label-fix">The bypass:</span>
					<span class="sw-frag">{strip(sel.bypassTitle)}</span>
				</h3>
				<p class="sw-copy">{sel.bypass}</p>
				<div class="sw-code">
					<AuraCode source={sel.code} />
				</div>
			</div>
		</div>
	{/key}
</div>

<style>
	.switchboard {
		border-top: 1px solid var(--color-rule);
	}

	.sw-tabs {
		display: flex;
		flex-wrap: wrap;
		gap: var(--space-lg);
		border-bottom: 1px solid var(--color-rule);
		padding-bottom: var(--space-sm);
	}

	.sw-tab {
		display: inline-flex;
		align-items: baseline;
		gap: var(--space-xs);
		font-size: var(--text-sm);
		color: var(--color-faint);
		transition: color var(--dur-micro) ease;
	}

	.sw-tab:hover {
		color: var(--color-ink);
	}

	.sw-tab-active {
		color: var(--color-ink);
		text-decoration: underline;
		text-decoration-color: var(--color-accent);
		text-decoration-thickness: 1px;
		text-underline-offset: 7px;
	}

	.sw-index {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.sw-body {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
		gap: clamp(32px, 5vw, 72px);
		padding-top: var(--space-xl);
	}

	.sw-col {
		min-width: 0;
	}

	.sw-label {
		display: block;
		font-family: var(--font-body);
		font-size: var(--text-xs);
		font-weight: 600;
		line-height: 1.6;
	}

	.sw-frag {
		display: block;
	}

	.sw-label-tax {
		color: var(--color-error);
	}

	.sw-label-fix {
		color: var(--color-signal);
	}

	.sw-title {
		font-family: var(--font-display);
		font-size: clamp(1.5rem, 2vw, 1.95rem);
		line-height: 1.15;
		color: var(--color-ink);
	}

	.sw-copy {
		margin-top: var(--space-sm);
		max-width: 52ch;
		font-size: var(--text-sm);
		line-height: 1.7;
		color: var(--color-muted);
	}

	.sw-code {
		margin-top: var(--space-md);
		overflow: hidden;
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
	}

	@media (max-width: 820px) {
		.sw-body {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
