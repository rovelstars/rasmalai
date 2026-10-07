<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import PlaygroundEditor from '$lib/components/PlaygroundEditor.svelte';
	import PlaygroundFiles from '$lib/components/PlaygroundFiles.svelte';
	import PlaygroundShell from '$lib/components/PlaygroundShell.svelte';
	import PlaygroundTerminal, { type TermStatus } from '$lib/components/PlaygroundTerminal.svelte';
	import { engine } from '$lib/playground/engine.svelte';
	import { encodeSnippet, decodeSnippet } from '$lib/playground/share';
	import { PRESETS } from '$lib/playground/presets';
	import { parseRunOutput } from '$lib/playground/shell';
	import {
		createProject,
		createSingleFileProject,
		getActiveContent,
		migrateLegacyTabs,
		projectFilesJson,
		serializeProject,
		setActiveContent,
		type IdeProject
	} from '$lib/playground/project-model';
	import { loadIdeState, mergeForLoad, saveIdeState } from '$lib/playground/ide-store';

	let presetIndex = $state(0);
	let projects = $state<IdeProject[]>([]);
	let activeId = $state<string | null>(null);
	let code = $state('');
	let status = $state<TermStatus>('idle');
	let diagnostics = $state('');
	let stdout = $state<string[]>([]);
	let retval = $state('');
	let duration = $state<number | null>(null);
	let notice = $state('');
	let shared = $state(false);
	let ranSource = $state<string | null>(null);
	let editorRef = $state<{ formatDocument: () => Promise<void> } | null>(null);

	const TABS_KEY = 'rnx-playground-tabs';
	let saveTimer: ReturnType<typeof setTimeout> | null = null;
	let checkTimer: ReturnType<typeof setTimeout> | null = null;

	function activeProject(): IdeProject | null {
		return projects.find((p) => p.id === activeId) ?? null;
	}

	let ap = $derived(activeProject());

	function persistSoon(): void {
		if (saveTimer) clearTimeout(saveTimer);
		saveTimer = setTimeout(() => {
			void saveIdeState({ projects: projects.map(serializeProject), activeId });
		}, 800);
	}

	function syncCodeFromProject(): void {
		const p = activeProject();
		code = p ? getActiveContent(p) : '';
	}

	function switchProject(id: string): void {
		const p = projects.find((x) => x.id === id);
		if (!p) return;
		activeId = id;
		syncCodeFromProject();
		ranSource = null;
		clearOutput('No diagnostics. Press Run or Cmd+Enter.');
		scheduleCheck();
		persistSoon();
	}

	function addProject(): void {
		const p = createProject(`Project ${projects.length + 1}`);
		projects.push(p);
		activeId = p.id;
		syncCodeFromProject();
		ranSource = null;
		clearOutput('Empty project. Press Run or Cmd+Enter.');
		scheduleCheck();
		persistSoon();
	}

	function deleteProject(id: string): void {
		if (projects.length <= 1) return;
		projects = projects.filter((p) => p.id !== id);
		if (activeId === id) {
			activeId = projects[0].id;
			syncCodeFromProject();
			ranSource = null;
			clearOutput('No diagnostics. Press Run or Cmd+Enter.');
			scheduleCheck();
		}
		persistSoon();
	}

	function onProjectChanged(): void {
		const p = activeProject();
		if (!p) return;
		const fresh = getActiveContent(p);
		if (fresh !== code) code = fresh;
		scheduleCheck();
		persistSoon();
	}

	let stale = $derived(ranSource !== null && ranSource !== code);

	$effect(() => {
		const p = activeProject();
		if (p && getActiveContent(p) !== code) {
			setActiveContent(p, code);
			persistSoon();
		}
		scheduleCheck();
	});

	function selectPreset(i: number) {
		presetIndex = i;
		const p = activeProject();
		if (!p) return;
		setActiveContent(p, PRESETS[i].source);
		code = PRESETS[i].source;
		ranSource = null;
		clearOutput('Preset loaded. Press Run or Cmd+Enter.');
		scheduleCheck();
		persistSoon();
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
			if (!engine.loaded || engine.state === 'running' || status === 'running') return;
			const p = activeProject();
			if (!p) return;
			status = 'checking';
			try {
				const { json, entry } = projectFilesJson(p);
				const text = await engine.checkProject(json, entry);
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
		const p = activeProject();
		if (!p || engine.state === 'running' || status === 'running') return;
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
		const { json, entry } = projectFilesJson(p);
		const diag = await engine.checkProject(json, entry).catch(() => '');
		if (diag) {
			diagnostics = diag;
			status = 'error';
			return;
		}
		let res;
		try {
			res = await engine.runProject(json, entry);
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
		const parsed = parseRunOutput(res.output);
		stdout = parsed.stdoutLines;
		retval = parsed.retval;
		if (parsed.diagnostics) {
			diagnostics = parsed.diagnostics;
			status = 'error';
		}
		if (!diagnostics && stdout.length === 0 && !retval) notice = 'Ran with no output.';
	}

	async function formatActive(): Promise<void> {
		if (!editorRef) return;
		try {
			await editorRef.formatDocument();
			notice = 'Formatted.';
		} catch (e) {
			diagnostics = e instanceof Error ? e.message : 'Format failed.';
			status = 'error';
		}
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
		void (async () => {
			if (incoming !== null && incoming.length > 0) {
				const p = createSingleFileProject('Shared snippet', incoming);
				projects = [p];
				activeId = p.id;
				code = incoming;
				notice = 'Snippet loaded from shared link as a single-file project.';
			} else {
				const saved = await loadIdeState().catch(() => null);
				const merged = mergeForLoad(saved);
				if (merged.projects.length > 0) {
					projects = merged.projects;
					activeId = merged.activeId;
					syncCodeFromProject();
					notice = 'Restored your last session.';
				} else {
					let legacy: unknown = null;
					try {
						const raw = localStorage.getItem(TABS_KEY);
						if (raw) legacy = JSON.parse(raw);
					} catch {
						legacy = null;
					}
					const tabs = (legacy as { tabs?: { name?: unknown; code?: unknown }[] } | null)?.tabs;
					const migrated = Array.isArray(tabs) ? migrateLegacyTabs(tabs) : null;
					if (migrated) {
						projects = [migrated];
						activeId = migrated.id;
						syncCodeFromProject();
						notice = 'Migrated snippet tabs into a project.';
					} else {
						const p = createSingleFileProject('Playground', PRESETS[0].source);
						projects = [p];
						activeId = p.id;
						code = PRESETS[0].source;
						notice = 'Engine unloaded. Press Run to download the compiler (~1MB, cached).';
					}
				}
			}
			scheduleCheck();
		})();
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
		<label class="ml-2 font-mono text-xs text-aura-muted" for="project">project</label>
		<select
			id="project"
			class="max-w-44 rounded-md border border-aura-border bg-aura-surface px-2.5 py-1.5 text-sm"
			value={activeId}
			onchange={(e) => switchProject((e.target as HTMLSelectElement).value)}
		>
			{#each projects as p}
				<option value={p.id}>{p.name}</option>
			{/each}
		</select>
		<button
			onclick={addProject}
			class="press min-h-8 rounded border border-aura-border px-2 py-1.5 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
			aria-label="New project"
		>
			+
		</button>
		{#if projects.length > 1 && activeId}
			<button
				onclick={() => activeId && deleteProject(activeId)}
				class="press min-h-8 rounded border border-aura-border px-2 py-1.5 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-red"
				aria-label="Delete project"
			>
				del
			</button>
		{/if}
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
				onclick={formatActive}
				class="press min-h-8 rounded-md border border-aura-border px-3 py-1.5 text-sm text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
			>
				format
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
		<div class="flex min-w-0 flex-col gap-4">
			<div class="panel min-w-0 overflow-hidden">
				{#if ap}
					<div class="border-b border-aura-border px-3 py-1.5 font-mono text-xs text-aura-muted">
						{ap.activePath} <span class="opacity-60">- entry {ap.entry}</span>
					</div>
					<PlaygroundEditor
						bind:this={editorRef}
						bind:value={code}
						onRun={run}
						filePath={ap.activePath.replace(/^\//, '')}
						getProject={() => {
							const cur = activeProject();
							return cur ? projectFilesJson(cur) : null;
						}}
					/>
				{/if}
			</div>
			{#if ap}
				<div class="panel max-h-72 min-w-0 overflow-hidden">
					<PlaygroundFiles project={ap} onChanged={onProjectChanged} />
				</div>
			{/if}
		</div>
		<div class="flex min-w-0 flex-col gap-4">
			<PlaygroundTerminal {status} {diagnostics} {stdout} {retval} {duration} {notice} {stale} />
			{#if ap}
				<PlaygroundShell project={ap} onExecuted={onProjectChanged} />
			{/if}
		</div>
	</div>
	<p class="mt-3 font-mono text-xs text-aura-muted">
		Runs locally in your browser (WebAssembly + Web Worker). Infinite loops are terminated
		after 3000 ms. Share links encode the active file in the URL hash; no code leaves your machine.
		Projects persist in IndexedDB. The shell runs rnx check/run/fmt/test on the WebAssembly engine only.
	</p>
</main>
