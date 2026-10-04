<script lang="ts">
	import { Play, RotateCcw, Zap } from 'lucide-svelte';
	import AnsiDemo from '$lib/components/AnsiDemo.svelte';
	import AuraCode from '$lib/components/AuraCode.svelte';
	import { fetchDataFile } from '$lib/docs/data-url';
	import { onMount } from 'svelte';

	// Legacy unversioned fallback; resolution prefers the deploy-versioned
	// copy via /data/version.json (see data-url.ts).
	export const demosUrl = '/data/cli-demos.json';

	let checkOutput = $state<string | null>(null);

	onMount(async () => {
		try {
			const res = await fetchDataFile(fetch, 'cli-demos.json');
			if (res) checkOutput = (await res.json()).check ?? null;
		} catch {
			checkOutput = null;
		}
	});

	type Tab = 'check' | 'test' | 'lint' | 'ffi';
	let tab = $state<Tab>('check');

	const TABS: { id: Tab; n: string; label: string }[] = [
		{ id: 'check', n: '1', label: 'Diagnostics (check)' },
		{ id: 'test', n: '2', label: 'Isolated Tests (test)' },
		{ id: 'lint', n: '3', label: 'Self-Healing (lint)' },
		{ id: 'ffi', n: '4', label: 'Native C-ABI (ffi)' }
	];

	let testLines = $state<string[]>([]);
	let testRunning = $state(false);
	let testDone = $state(false);

	let fixed = $state(false);
	let fixFlash = $state(false);

	const reduced =
		typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;

	function runTests() {
		if (testRunning) return;
		testRunning = true;
		testDone = false;
		testLines = [];
		const steps = ['✓ adds_up 0.01 ms', '✓ shadows 0.00 ms', 'tests: 2 passed, 0 failed in 0.08 ms'];
		if (reduced) {
			testLines = steps;
			testRunning = false;
			testDone = true;
			return;
		}
		steps.forEach((s, i) => {
			setTimeout(() => {
				testLines = [...testLines, s];
				if (i === steps.length - 1) {
					testRunning = false;
					testDone = true;
				}
			}, 350 * (i + 1));
		});
	}

	function autoFix() {
		if (fixed) return;
		fixed = true;
		if (!reduced) {
			fixFlash = true;
			setTimeout(() => (fixFlash = false), 900);
		}
	}

	function switchTab(t: Tab) {
		tab = t;
		if (t !== 'test') {
			testLines = [];
			testDone = false;
		}
		if (t !== 'lint') fixed = false;
	}

	const abiFrame = `// no wrappers, no boilerplate
import {
    fn compressBound(n: Int): Int
} from native "z"`;
</script>

<div class="bay" data-testid="toolchain-station">
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-aura-border px-4 py-2 font-mono text-[11px]">
		<span class="text-aura-muted">toolchain station - one binary: rnx</span>
		<span class="ml-auto hidden text-aura-muted sm:inline">[ BUS: rnx check - test - lint - ffi ]</span>
	</div>

	<div
		class="flex gap-1 overflow-x-auto border-b border-aura-border p-2"
		role="tablist"
		aria-label="Toolchain modes"
	>
		{#each TABS as t}
			<button
				role="tab"
				aria-selected={tab === t.id}
				onclick={() => switchTab(t.id)}
				class="press shrink-0 rounded-full border px-3.5 py-1.5 font-mono text-[12px] whitespace-nowrap {tab === t.id
					? 'border-aura-purple/50 bg-aura-purple/10 text-aura-purple'
					: 'border-aura-border text-aura-muted hover:border-aura-borderHover hover:text-aura-text'}"
			>
				<span class="mr-1.5 inline-block h-1.5 w-1.5 rounded-full bg-aura-purple {tab === t.id ? '' : 'opacity-50'}"></span>{t.n}. {t.label}
			</button>
		{/each}
	</div>

	<div class="min-h-[300px]">
		{#if tab === 'check'}
			{#if checkOutput !== null}
				<AnsiDemo output={checkOutput} command="rnx check src/bad.rnx" />
			{:else}
				<p class="p-4 font-mono text-[12px] text-aura-muted">
					$ rnx check src/bad.rnx <span class="text-aura-muted">— demo output loads with site data.</span>
				</p>
			{/if}
			<p class="px-4 pb-4 font-mono text-[12px] text-aura-muted">
				tree-hook diagnostics point at the exact column and suggest the fix. Try it above in
				the workbench — edit the source until it breaks.
			</p>
		{:else if tab === 'test'}
			<div class="p-4 font-mono text-[12px] leading-7">
				<p class="text-aura-muted">$ rnx test <span class="text-aura-muted">— tests/demo.rnx</span></p>
				{#each testLines as line}
					<p class={line.startsWith('tests:') ? 'text-aura-text/80' : 'text-aura-green'}>
						{line}
					</p>
				{/each}
				{#if !testDone && !testRunning}
					<p class="text-aura-muted">suite armed — press run.</p>
				{/if}
				<div class="mt-3">
					<button
						onclick={runTests}
						disabled={testRunning}
						class="press flex items-center gap-2 rounded bg-aura-purple px-4 py-1.5 font-mono text-[12px] font-bold text-aura-bg disabled:opacity-50"
					>
						{#if testRunning}... running{:else if testDone}<RotateCcw size={13} /> Run Again{:else}<Play size={13} /> Run Test Suite{/if}
					</button>
				</div>
				<p class="mt-3 text-aura-muted">sample replay — each file runs isolated, so one crash can't hide another.</p>
			</div>
		{:else if tab === 'lint'}
			<div class="p-4 font-mono text-[12px] leading-7">
				<p class="text-aura-muted">$ rnx lint tests/demo.rnx</p>
				<p class="text-aura-text/70">
					<span class="text-aura-muted">tests/demo.rnx:6:5:</span>
					{#if fixed}
						<span class={fixFlash ? 'bg-aura-green/20 text-aura-green' : 'text-aura-green'}>
							fixed — no warnings
						</span>
					{:else}
						<span class="text-aura-orange">warning[L001]: unused variable `x`</span>
					{/if}
				</p>
				<p class="overflow-x-auto whitespace-pre text-aura-text/80"> 6 |     let {fixed ? '_x' : 'x'} = x + 1;</p>
				{#if !fixed}
					<p class="text-aura-muted">hint: prefix `x` with `_` to silence</p>
					<div class="mt-3">
						<button
							onclick={autoFix}
							class="press flex items-center gap-2 rounded bg-aura-purple px-4 py-1.5 font-mono text-[12px] font-bold text-aura-bg"
						>
							<Zap size={13} /> Click to Auto-Fix
						</button>
					</div>
				{:else}
					<p class="mt-3 text-aura-green">diff applied — you just review.</p>
				{/if}
				<p class="mt-3 text-aura-muted">mirrors L001 on the demo file — the real fixer runs in rnx lint.</p>
			</div>
		{:else}
			<AuraCode source={abiFrame} />
			<p class="px-4 pb-4 font-mono text-[12px] text-aura-muted">
				declare the symbols you need and rnx links them — no headers, no binding generators.
			</p>
		{/if}
	</div>
</div>
