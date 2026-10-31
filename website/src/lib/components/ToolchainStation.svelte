<script lang="ts">
	import { Play, RotateCcw } from 'lucide-svelte';
	import AnsiDemo from '$lib/components/AnsiDemo.svelte';
	import AuraCode from '$lib/components/AuraCode.svelte';
	import { fetchDataFile } from '$lib/docs/data-url';
	import { engine } from '$lib/playground/engine.svelte';
	import { onMount } from 'svelte';

	// Legacy unversioned fallback; resolution prefers the deploy-versioned
	// copy via /data/version.json (see data-url.ts).
	export const demosUrl = '/data/cli-demos.json';

	let checkOutput = $state<string | null>(null);
	let lintBefore = $state<string | null>(null);
	let lintAfter = $state<string | null>(null);

	onMount(async () => {
		try {
			const res = await fetchDataFile(fetch, 'cli-demos.json');
			if (!res) return;
			const data = await res.json();
			checkOutput = data.check ?? null;
			lintBefore = data.lint ?? null;
			lintAfter = data.lintFix ?? null;
		} catch {
			checkOutput = null;
			lintBefore = null;
			lintAfter = null;
		}
	});

	type Tab = 'check' | 'test' | 'lint' | 'ffi';
	let tab = $state<Tab>('check');

	const TABS: { id: Tab; n: string; label: string }[] = [
		{ id: 'check', n: '01', label: 'Diagnostics' },
		{ id: 'test', n: '02', label: 'Isolated tests' },
		{ id: 'lint', n: '03', label: 'Self-healing' },
		{ id: 'ffi', n: '04', label: 'Native C-ABI' }
	];

	let testLines = $state<string[]>([]);
	let testRunning = $state(false);
	let testDone = $state(false);
	let testError = $state(false);

	const TEST_PROJECT = JSON.stringify({
		'tests/demo.rnx': `test fn adds_up() {
    assert(1 + 1 == 2, "math");
}

test fn shadows() {
    let x = 1;
    let x = x + 1;
    assert(x == 2, "shadow");
}
`
	});

	async function runTests() {
		if (testRunning) return;
		testRunning = true;
		testDone = false;
		testError = false;
		testLines = [];
		try {
			const res = await engine.testProject(TEST_PROJECT, 'tests/demo.rnx');
			if (res.timedOut) {
				testError = true;
				testLines = ['test run exceeded 3000 ms and was stopped'];
				return;
			}
			testLines = res.output.split('\n').filter((line) => line.length > 0);
		} catch (e) {
			testError = true;
			testLines = [e instanceof Error ? e.message : 'test run failed'];
		} finally {
			testRunning = false;
			testDone = true;
		}
	}

	function switchTab(t: Tab) {
		tab = t;
		if (t !== 'test') {
			testLines = [];
			testDone = false;
			testError = false;
		}
	}

	const abiFrame = `// no wrappers, no boilerplate
import {
    fn compressBound(n: Int): Int
} from native "z"`;
</script>

<div class="station" data-testid="toolchain-station">
	<div class="station-tabs" role="tablist" aria-label="Toolchain modes">
		{#each TABS as t}
			<button
				role="tab"
				aria-selected={tab === t.id}
				onclick={() => switchTab(t.id)}
				class="station-tab"
				class:station-tab-active={tab === t.id}
			>
				<span class="station-n">{t.n}</span>
				{t.label}
			</button>
		{/each}
	</div>

	<div class="station-body">
		{#if tab === 'check'}
			{#if checkOutput !== null}
				<AnsiDemo output={checkOutput} command="rnx check src/bad.rnx" />
			{:else}
				<p class="line muted">demo output for $ rnx check src/bad.rnx loads with site data.</p>
			{/if}
			<p class="note">
				Captured from the shipped CLI. Tree-hook diagnostics point at the exact column and
				suggest the fix; the workbench above runs the same checker in the browser.
			</p>
		{:else if tab === 'test'}
			<div class="terminal">
				<p class="line muted">$ rnx test <span class="muted">(browser engine)</span></p>
				{#each testLines as line}
					<p class="line {testError ? 'text-aura-red' : line.startsWith('tests:') ? 'text-aura-text/80' : 'text-aura-green'}">{line}</p>
				{/each}
				{#if testRunning}
					<p class="line muted">loading the engine and running the suite…</p>
				{:else if !testDone}
					<p class="line muted">two demo tests, press run.</p>
				{/if}
				<div class="mt-4">
					<button onclick={runTests} disabled={testRunning} class="btn btn-primary h-9 min-h-0 px-4 text-[13px]">
						{#if testRunning}running…{:else if testDone}<RotateCcw size={13} /> Run again{:else}<Play size={13} /> Run test suite{/if}
					</button>
				</div>
				<p class="note">
					Runs the two demo tests in the browser WebAssembly engine; the first run downloads
					it. Each test runs isolated, so one failure does not hide another.
				</p>
			</div>
		{:else if tab === 'lint'}
			<div class="space-y-3">
				{#if lintBefore !== null}
					<AnsiDemo output={lintBefore} command="rnx lint tests/demo.rnx" />
					{#if lintAfter !== null}
						<AnsiDemo output={lintAfter} command="rnx lint --fix tests/demo.rnx" />
					{/if}
				{:else}
					<p class="line muted">captured lint output loads with site data.</p>
				{/if}
				<p class="note">
					Captured from the shipped CLI. The fixer rewrites this L001 site in place and the
					second run is clean; nothing is simulated in the browser.
				</p>
			</div>
		{:else}
			<div class="terminal">
				<div class="overflow-hidden rounded-sm border border-aura-border">
					<AuraCode source={abiFrame} />
				</div>
				<p class="note">declare the symbols you need and rnx links them: no headers, no binding generators.</p>
			</div>
		{/if}
	</div>
</div>

<style>
	.station {
		border-top: 1px solid var(--color-rule);
	}

	.station-tabs {
		display: flex;
		flex-wrap: wrap;
		gap: var(--space-lg);
		border-bottom: 1px solid var(--color-rule);
		padding-bottom: var(--space-sm);
	}

	.station-tab {
		display: inline-flex;
		align-items: baseline;
		gap: var(--space-xs);
		font-size: var(--text-sm);
		color: var(--color-faint);
		transition: color var(--dur-micro) ease;
	}

	.station-tab:hover {
		color: var(--color-ink);
	}

	.station-tab-active {
		color: var(--color-ink);
		text-decoration: underline;
		text-decoration-color: var(--color-accent);
		text-decoration-thickness: 1px;
		text-underline-offset: 7px;
	}

	.station-n {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.station-body {
		min-height: 280px;
		padding-top: var(--space-md);
	}

	.terminal {
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		background: var(--color-paper-2);
		padding: var(--space-md) var(--space-lg);
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		line-height: 1.9;
	}

	.line {
		margin: 0;
	}

	.muted {
		color: var(--color-muted);
	}

	.note {
		margin-top: var(--space-sm);
		font-family: var(--font-body);
		font-size: var(--text-xs);
		line-height: 1.6;
		color: var(--color-faint);
	}
</style>
