<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import PlaygroundEditor from '$lib/components/PlaygroundEditor.svelte';
	import { X } from 'lucide-svelte';
	import PlaygroundTerminal, { type TermStatus } from '$lib/components/PlaygroundTerminal.svelte';
	import { engine } from '$lib/playground/engine.svelte';
	import { encodeSnippet, decodeSnippet } from '$lib/playground/share';
	import { PRESETS } from '$lib/playground/presets';

	let presetIndex = $state(0);
	let code = $state(PRESETS[0].source);
	let status = $state<TermStatus>('idle');
	let diagnostics = $state('');
	let stdout = $state<string[]>([]);
	let retval = $state('');
	let duration = $state<number | null>(null);
	let notice = $state('');
	let shared = $state(false);
	let ranSource = $state<string | null>(null);

	interface Tab {
		id: string;
		name: string;
		code: string;
	}
	let tabs = $state<Tab[]>([{ id: 't1', name: 'Snippet 1', code: PRESETS[0].source }]);
	let activeId = $state('t1');
	let tabSeq = $state(2);
	let renamingId = $state<string | null>(null);

	const TABS_KEY = 'rnx-playground-tabs';
	let saveTimer: ReturnType<typeof setTimeout> | null = null;

	function persistTabs() {
		try {
			localStorage.setItem(TABS_KEY, JSON.stringify({ tabs, activeId }));
		} catch {
			/* storage unavailable */
		}
	}

	function loadTabs(): boolean {
		try {
			const raw = localStorage.getItem(TABS_KEY);
			if (!raw) return false;
			const data = JSON.parse(raw) as { tabs?: Tab[]; activeId?: string };
			if (!Array.isArray(data.tabs) || data.tabs.length === 0) return false;
			const clean = data.tabs
				.filter((t) => t && typeof t.code === 'string')
				.map((t, i) => ({
					id: typeof t.id === 'string' ? t.id : `t${i + 1}`,
					name: typeof t.name === 'string' && t.name ? t.name.slice(0, 24) : `Snippet ${i + 1}`,
					code: t.code
				}));
			if (clean.length === 0) return false;
			tabs = clean;
			tabSeq = clean.length + 1;
			activeId = clean.some((t) => t.id === data.activeId) ? (data.activeId as string) : clean[0].id;
			return true;
		} catch {
			return false;
		}
	}

	function activeTab(): Tab {
		return tabs.find((t) => t.id === activeId) ?? tabs[0];
	}

	function switchTab(id: string) {
		const tab = tabs.find((t) => t.id === activeId);
		if (tab && tab.code !== code) tab.code = code;
		activeId = id;
		code = activeTab().code;
		ranSource = null;
		clearOutput('No diagnostics. Press Run or Cmd+Enter.');
		scheduleCheck();
		persistTabs();
	}

	function addTab() {
		const tab = tabs.find((t) => t.id === activeId);
		if (tab && tab.code !== code) tab.code = code;
		const id = `t${tabSeq++}`;
		tabs.push({ id, name: `Snippet ${tabs.length + 1}`, code: PRESETS[0].source });
		activeId = id;
		code = PRESETS[0].source;
		ranSource = null;
		clearOutput('No diagnostics. Press Run or Cmd+Enter.');
		scheduleCheck();
		persistTabs();
	}

	function closeTab(id: string) {
		if (tabs.length <= 1) {
			const only = tabs[0];
			only.code = PRESETS[0].source;
			only.name = 'Snippet 1';
			code = only.code;
			ranSource = null;
			clearOutput('No diagnostics. Press Run or Cmd+Enter.');
			scheduleCheck();
			persistTabs();
			return;
		}
		const ix = tabs.findIndex((t) => t.id === id);
		tabs.splice(ix, 1);
		if (activeId === id) {
			const next = tabs[Math.min(ix, tabs.length - 1)];
			activeId = next.id;
			code = next.code;
			ranSource = null;
			clearOutput('No diagnostics. Press Run or Cmd+Enter.');
			scheduleCheck();
		}
		persistTabs();
	}

	let checkTimer: ReturnType<typeof setTimeout> | null = null;

	let stale = $derived(ranSource !== null && ranSource !== code);

	$effect(() => {
		void code;
		scheduleCheck();
		const tab = tabs.find((t) => t.id === activeId);
		if (tab && tab.code !== code) tab.code = code;
		if (saveTimer) clearTimeout(saveTimer);
		saveTimer = setTimeout(persistTabs, 500);
	});

	function selectPreset(i: number) {
		presetIndex = i;
		code = PRESETS[i].source;
		ranSource = null;
		clearOutput('Preset loaded. Press Run or Cmd+Enter.');
		scheduleCheck();
	}

	function clearOutput(msg: string) {
		diagnostics = '';
		stdout = [];
		retval = '';
		duration = null;
		notice = msg;
	}

	function scheduleCheck() {
		if (checkTimer) clearTimeout(checkTimer);
		checkTimer = setTimeout(async () => {
			// Live diagnostics only after explicit engine load (Run click);
			// typing alone must never trigger the 1.1MB download.
			if (!engine.loaded || engine.state === 'running' || status === 'running') return;
			status = 'checking';
			try {
				const text = await engine.check(code);
				if (status !== 'checking') return;
				diagnostics = text;
				status = text ? 'error' : 'idle';
				if (!text) {
					notice = ranSource !== null && ranSource !== code
						? 'Edited since last run — press Run or Cmd+Enter to re-execute.'
						: 'No diagnostics. Press Run or Cmd+Enter.';
				} else {
					notice = '';
				}
			} catch {
				if (status === 'checking') status = 'idle';
			}
		}, 350);
	}

	async function run() {
		if (engine.state === 'running' || status === 'running') return;
		status = 'running';
		notice = '';
		diagnostics = '';
		stdout = [];
		retval = '';
		duration = null;
		try {
			await engine.ensureLoaded((f, fromCache) => {
				notice = fromCache
					? 'Engine restored from local cache.'
					: f === null
						? 'Downloading compiler engine...'
						: `Downloading compiler engine... ${Math.round(f * 100)}%`;
			});
		} catch (e) {
			status = 'error';
			diagnostics = e instanceof Error ? e.message : 'Engine failed to load.';
			return;
		}
		const diag = await engine.check(code).catch(() => '');
		if (diag) {
			diagnostics = diag;
			status = 'error';
			return;
		}
		let res;
		try {
			res = await engine.run(code);
		} catch (e) {
			diagnostics = e instanceof Error ? e.message : 'Run failed.';
			status = 'error';
			return;
		}
		ranSource = code;
		if (res.timedOut) {
			status = 'timeout';
			notice = 'Run exceeded 3000 ms and was terminated. The worker was restarted; no browser state was harmed.';
			duration = res.duration;
			return;
		}
		status = 'done';
		duration = res.duration;
		const lines = res.output.split('\n');
		const out: string[] = [];
		for (const line of lines) {
			if (line.startsWith('=> ')) retval = line;
			else if (line.startsWith('thrown: ') || line.startsWith('fatal: ')) {
				diagnostics = line;
				status = 'error';
			} else if (line.length > 0) out.push(line);
		}
		stdout = out;
		if (!diagnostics && out.length === 0 && !retval) notice = 'Ran with no output.';
	}

	async function share() {
		const url = window.location.origin + window.location.pathname + '#' + encodeSnippet(code);
		window.location.hash = encodeSnippet(code);
		try {
			await navigator.clipboard.writeText(url);
			shared = true;
			setTimeout(() => (shared = false), 1500);
		} catch {
			shared = false;
		}
	}

	onMount(() => {
		const fromQuery = page.url.searchParams.get('code');
		const fromHash = window.location.hash ? decodeSnippet(window.location.hash) : null;
		const fromLink = fromQuery ? decodeSnippet(fromQuery) : null;
		const incoming = fromLink ?? fromHash;
		if (incoming !== null && incoming.length > 0) {
			code = incoming;
			const tab = tabs.find((t) => t.id === activeId);
			if (tab) tab.code = code;
			notice = 'Snippet loaded from shared link.';
		} else if (loadTabs()) {
			code = activeTab().code;
			notice = 'Restored your last session.';
		} else {
			notice = 'Engine unloaded. Press Run to download the compiler (~1MB, cached).';
		}
		scheduleCheck();
		return () => {
			if (checkTimer) clearTimeout(checkTimer);
		};
	});
</script>

<svelte:head>
	<title>Playground — Rasmalai</title>
	<style>
		select option {
			background-color: #1c1b22;
		}
	</style>
</svelte:head>

<main class="mx-auto max-w-7xl px-4 pb-16">
	<div class="flex flex-wrap items-center gap-2 pt-8">
		<label class="font-mono text-xs text-aura-muted" for="preset">preset</label>
		<select
			id="preset"
			class="rounded-md border border-aura-border bg-aura-surface px-2.5 py-1.5 text-sm"
			value={presetIndex}
			onchange={(e) => selectPreset(Number((e.target as HTMLSelectElement).value))}
		>
			{#each PRESETS as p, i}
				<option value={i}>{p.name}</option>
			{/each}
		</select>
		<div class="ml-auto flex items-center gap-2">
			<button
				onclick={share}
				class="press min-h-8 rounded-md border border-aura-border px-3 py-1.5 text-sm text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
			>
				{shared ? 'link copied' : 'share'}
			</button>
			<button
				onclick={() => selectPreset(presetIndex)}
				class="press min-h-8 rounded-md border border-aura-border px-3 py-1.5 text-sm text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
			>
				reset
			</button>
			<button
				onclick={run}
				class="press min-h-8 rounded bg-aura-green px-4 py-1.5 font-mono text-sm font-medium text-[#15141b]"
			>
				Run <span class="opacity-60">(⌘+Enter)</span>
			</button>
		</div>
	</div>

	<div class="mt-4 grid gap-4 lg:grid-cols-2">
		<div class="panel min-w-0 overflow-hidden">
			<div class="flex items-center gap-1 overflow-x-auto border-b border-aura-border px-2 py-1" role="tablist" aria-label="Snippets">
				{#each tabs as t}
					<div
						role="tab"
						aria-selected={t.id === activeId}
						class="flex shrink-0 items-center gap-1.5 rounded px-2 py-1 font-mono text-xs {t.id === activeId
							? 'bg-aura-surfaceElevated text-aura-text'
							: 'text-aura-muted hover:text-aura-text'}"
					>
						{#if renamingId === t.id}
							<input
								value={t.name}
								onchange={(e) => {
									const v = (e.target as HTMLInputElement).value.trim().slice(0, 24);
									if (v) t.name = v;
									renamingId = null;
									persistTabs();
								}}
								onkeydown={(e) => {
									if (e.key === 'Enter' || e.key === 'Escape') (e.target as HTMLInputElement).blur();
								}}
								onblur={() => (renamingId = null)}
								class="w-24 bg-transparent outline-none"
								aria-label="Rename tab"
							/>
						{:else}
							<button
								onclick={() => switchTab(t.id)}
								ondblclick={() => (renamingId = t.id)}
								title="Double-click to rename"
								class="max-w-28 truncate"
							>
								{t.name}
							</button>
						{/if}
						<button
							onclick={() => closeTab(t.id)}
							class="flex min-h-[24px] min-w-[24px] items-center justify-center text-aura-muted hover:text-aura-red"
							aria-label="Close {t.name}"
						>
							<X size={12} />
						</button>
					</div>
				{/each}
				<button
					onclick={addTab}
					class="press shrink-0 rounded border border-aura-border px-2 py-0.5 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
					aria-label="New tab"
				>
					+
				</button>
			</div>
			<PlaygroundEditor
				bind:value={code}
				onRun={run}
			/>
		</div>
		<PlaygroundTerminal {status} {diagnostics} {stdout} {retval} {duration} {notice} {stale} />
	</div>
	<p class="mt-3 font-mono text-xs text-aura-muted">
		Runs locally in your browser (WebAssembly + Web Worker). Infinite loops are terminated
		after 3000 ms. Share links encode the snippet in the URL hash; no code leaves your machine.
	</p>
</main>
