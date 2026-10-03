<script lang="ts">
	import { Play, RotateCcw, Copy, Check, Pencil, ExternalLink, LoaderCircle } from 'lucide-svelte';
	import { highlightAura } from '$lib/aura/highlight';
	import { highlightCode } from '$lib/aura/highlight';
	import { engine } from '$lib/playground/engine.svelte';
	import { encodeSnippet } from '$lib/playground/share';

	export interface CodeTab {
		label: string;
		code: string;
		lang: string;
	}

	let {
		code,
		lang = 'rnx',
		tabs = [],
		startTab = 0
	}: { code: string; lang?: string; tabs?: CodeTab[]; startTab?: number } = $props();

	let active = $state(0);
	$effect(() => {
		active = Math.min(Math.max(startTab, 0), tabs.length);
	});
	let allTabs = $derived([{ label: 'Rasmalai', code, lang }, ...tabs]);
	let current = $derived(allTabs[active] ?? allTabs[0]);

	let edits = $state<string | null>(null);
	let shown = $derived(active === 0 ? (edits ?? code) : current.code);
	let editing = $state(false);
	let running = $state(false);
	let progress = $state('');
	let copied = $state(false);
	let stdout = $state<string[]>([]);
	let diagnostics = $state('');
	let retval = $state('');
	let duration = $state<number | null>(null);
	let drawerOpen = $state(false);
	let closing = $state(false);
	let closeTimer: ReturnType<typeof setTimeout> | undefined = undefined;

	function closeDrawer() {
		if (!drawerOpen || closing) return;
		closing = true;
		clearTimeout(closeTimer);
		closeTimer = setTimeout(() => {
			drawerOpen = false;
			closing = false;
		}, 300);
	}

	let edited = $derived(edits !== null && edits !== code);

	function escHtml(s: string): string {
		return s
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;');
	}

	function highlightLang(raw: string): string {
		if (raw.toLowerCase() === 'c++') return 'cpp';
		return raw;
	}

	let highlighted = $derived.by(() => {
		if (current.lang === 'rnx') {
			return highlightAura(shown)
				.map((s) => `<span class="${s.cls}">${escHtml(s.text)}</span>`)
				.join('');
		}
		const spans = highlightCode(shown, highlightLang(current.lang));
		if (!spans) return escHtml(shown);
		return spans.map((s) => `<span class="${s.cls}">${escHtml(s.text)}</span>`).join('');
	});

	function onTab(e: KeyboardEvent) {
		if (e.key !== 'Tab') return;
		e.preventDefault();
		const t = e.target as HTMLTextAreaElement;
		const s = t.selectionStart ?? shown.length;
		const en = t.selectionEnd ?? s;
		edits = shown.slice(0, s) + '    ' + shown.slice(en);
		requestAnimationFrame(() => t.setSelectionRange(s + 4, s + 4));
	}

	async function copy() {
		try {
			await navigator.clipboard.writeText(shown);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			copied = false;
		}
	}

	function reset() {
		edits = null;
		editing = false;
	}

	async function run() {
		if (running) return;
		running = true;
		clearTimeout(closeTimer);
		closing = false;
		drawerOpen = true;
		duration = null;
		progress = engine.loaded ? 'Checking...' : 'Downloading compiler engine...';
		try {
			await engine.ensureLoaded((f, fromCache) => {
				progress = fromCache
					? 'Engine restored from local cache.'
					: f === null
						? 'Downloading compiler engine...'
						: `Downloading compiler engine... ${Math.round(f * 100)}%`;
			});
			progress = 'Checking...';
			const diag = await engine.check(shown).catch(() => '');
			if (diag) {
				stdout = [];
				retval = '';
				diagnostics = diag;
				progress = '';
				running = false;
				return;
			}
			progress = 'Running...';
			const res = await engine.run(shown);
			duration = res.duration;
			stdout = [];
			retval = '';
			diagnostics = '';
			if (res.timedOut) {
				diagnostics = 'Run exceeded 3000 ms and was terminated.';
			} else {
				for (const line of res.output.split('\n')) {
					if (line.startsWith('=> ')) retval = line;
					else if (line.startsWith('thrown: ') || line.startsWith('fatal: ')) diagnostics = line;
					else if (line.length > 0) stdout = [...stdout, line];
				}
			}
		} catch (e) {
			diagnostics = e instanceof Error ? e.message : 'Run failed.';
		} finally {
			progress = '';
			running = false;
		}
	}
</script>

<div class="doc-codehead">
	{#if allTabs.length > 1}
		<span role="tablist" aria-label="Language comparison" class="flex items-center gap-1">
			{#each allTabs as t, i}
				<button
					role="tab"
					aria-selected={active === i}
					onclick={() => (active = i)}
					class="doc-btn {active === i ? 'doc-tab-active' : ''}"
				>
					{t.label}
				</button>
			{/each}
		</span>
	{:else}
		<span class="doc-lang">.rnx</span>
	{/if}
	{#if edited}<span class="doc-edited">edited</span>{/if}
	<span class="doc-actions">
		{#if current.lang === 'rnx'}
			{#if editing}
				<button onclick={() => (editing = false)} class="doc-btn" aria-label="Done editing">
					<Check size={13} />done
				</button>
			{:else}
				<button onclick={() => (editing = true)} class="doc-btn" aria-label="Edit snippet">
					<Pencil size={13} />edit
				</button>
			{/if}
		{/if}
		{#if edited}
			<button onclick={reset} class="doc-btn" aria-label="Reset snippet">
				<RotateCcw size={13} />reset
			</button>
		{/if}
		<button onclick={copy} class="doc-btn min-w-[78px] justify-center" aria-label="Copy snippet">
			{#if copied}<Check size={13} />copied{:else}<Copy size={13} />copy{/if}
		</button>
		{#if current.lang === 'rnx'}
			<a
				href="/playground?code={encodeSnippet(shown)}"
				target="_blank"
				rel="noopener"
				class="doc-btn"
				aria-label="Open in playground"
			>
				<ExternalLink size={13} />open
			</a>
			<button onclick={run} disabled={running} class="doc-btn doc-run min-w-[76px] justify-center" aria-label="Run snippet">
				{#if running}<LoaderCircle size={13} class="animate-spin" />{:else}<Play size={13} />{/if}run
			</button>
		{/if}
	</span>
</div>
{#if editing && current.lang === 'rnx'}
	<textarea
		value={shown}
		oninput={(e) => (edits = (e.target as HTMLTextAreaElement).value)}
		onkeydown={onTab}
		spellcheck={false}
		aria-label="Editable Rasmalai snippet"
		class="doc-edit"
		rows={Math.min(24, Math.max(4, shown.split('\n').length + 1))}
	></textarea>
{:else}
	<pre><code>{@html highlighted}</code></pre>
{/if}
<div class="doc-drawer-wrap {drawerOpen && !closing ? 'open' : ''}">
	<div class="doc-drawer-inner">
		{#if drawerOpen}
			<div class="doc-drawer drawer-in min-h-11" aria-live="polite">
				{#if running && !diagnostics && stdout.length === 0 && !retval}
					<p class="text-aura-muted">{progress || 'Running...'}</p>
				{:else}
					{#if diagnostics}
						<p class="doc-drawer-err">{diagnostics}</p>
					{/if}
					{#if stdout.length > 0}
						<p class="doc-drawer-out">{stdout.join('\n')}</p>
					{/if}
					{#if retval}
						<p class="text-aura-green">{retval}</p>
					{/if}
					{#if !diagnostics && stdout.length === 0 && !retval}
						<p class="text-aura-muted">Ran with no output.</p>
					{/if}
				{/if}
					<div class="doc-drawer-meta">
						{#if running}
							<span class="text-aura-muted">{progress || 'Running...'}</span>
						{:else}
							{#if duration !== null}<span>took {duration.toFixed(1)} ms</span>{/if}
							<span>{diagnostics ? 'exit: error' : 'exit: ok'}</span>
						{/if}
							<button onclick={closeDrawer} class="doc-btn" aria-label="Close output">
								close
							</button>
					</div>
			</div>
		{/if}
	</div>
</div>
