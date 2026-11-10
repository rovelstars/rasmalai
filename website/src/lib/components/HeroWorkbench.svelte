<script lang="ts">
	import { LoaderCircle, RotateCw } from 'lucide-svelte';
	import { engine } from '$lib/playground/engine.svelte';

	let source = $state(
		`import { Vec4f } from "@std/simd";

fn shade(sun: Vec4f): Float {
    defer { println!("shade done"); }
    let acc = Vec4f.splat(0.0);
    let v = new Vec4f(1.0, 2.0, 3.0, 4.0);
    let lit = v * sun;
    return acc.dot(lit);
}`
	);
	let jitMs = $state<number | null>(null);
	let diag = $state('Hit Re-JIT to download the compiler engine and check your code.');
	let checking = $state(false);
	let debounce: ReturnType<typeof setTimeout> | undefined = undefined;

	function esc(s: string): string {
		return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
	}

	function highlight(line: string): string {
		return esc(line).replace(
			/(\/\/.*$)|("[^"]*")|\b(import|from|fn|let|return|defer)\b|\b(Vec4f|Float|Int|Str)\b|\b(\d+\.\d+|\d+)\b/g,
			(m, com, str, kw, typ, num) => {
				if (com) return `<span class="text-aura-faint italic">${m}</span>`;
				if (str) return `<span class="text-aura-orange">${m}</span>`;
				if (kw) return `<span class="text-aura-purple">${m}</span>`;
				if (typ) return `<span class="text-aura-pink">${m}</span>`;
				if (num) return `<span class="text-aura-orange">${m}</span>`;
				return m;
			}
		);
	}

	let lines = $derived(source.split('\n'));

	async function checkNow() {
		checking = true;
		try {
			if (!engine.loaded) diag = 'Downloading compiler engine...';
			await engine.ensureLoaded((f, fromCache) => {
				diag = fromCache
					? 'Engine restored from local cache.'
					: f === null
						? 'Downloading compiler engine...'
						: `Downloading compiler engine... ${Math.round(f * 100)}%`;
			});
			const t0 = performance.now();
			const tokJson = await engine.tokens(source);
			const tokMs = performance.now() - t0;
			const t1 = performance.now();
			const out = await engine.check(source);
			const checkMs = performance.now() - t1;
			const ms = tokMs + checkMs;
			jitMs = ms;
			if (out.trim() !== '') {
				diag = out;
				return;
			}
			const bytes = new TextEncoder().encode(source).length;
			let counts = new Map<string, number>();
			try {
				const parsed = JSON.parse(tokJson) as { tokens?: { role: string }[] };
				for (const t of parsed.tokens ?? []) counts.set(t.role, (counts.get(t.role) ?? 0) + 1);
			} catch {
				counts = new Map();
			}
			const total = [...counts.values()].reduce((a, b) => a + b, 0);
			const roles = [...counts.entries()]
				.sort((a, b) => b[1] - a[1])
				.map(([k, v]) => `${k} ${v}`)
				.join(' - ');
			diag =
				`check ok - ${ms.toFixed(2)} ms\n` +
				`${lines.length} lines - ${bytes} bytes - ${total} tokens${roles ? `\n${roles}` : ''}\n` +
				`tokenize ${tokMs.toFixed(2)} ms - parse+check ${checkMs.toFixed(2)} ms`;
		} catch (e) {
			diag = e instanceof Error ? e.message : 'compile pass failed';
		} finally {
			checking = false;
		}
	}

	function onEdit() {
		clearTimeout(debounce);
		// Live re-check only after the user explicitly loaded the engine
		// via Re-JIT; typing alone must never trigger the download.
		if (!engine.loaded) return;
		debounce = setTimeout(checkNow, 800);
	}

	function onTab(e: KeyboardEvent) {
		if (e.key !== 'Tab') return;
		e.preventDefault();
		const t = e.target as HTMLTextAreaElement;
		const s = t.selectionStart ?? source.length;
		source = source.slice(0, s) + '    ' + source.slice(t.selectionEnd ?? s);
		requestAnimationFrame(() => t.setSelectionRange(s + 4, s + 4));
	}
</script>

<div class="workbench" data-testid="hero-workbench">
	<div class="workbench-head">
		<span class="file">src/shade.rnx</span>
		<span class="hint">on-demand check</span>
		<div class="ml-auto flex items-center gap-3">
			<button onclick={checkNow} disabled={checking} class="run">
				{#if checking}<LoaderCircle size={12} class="animate-spin" />{:else}<RotateCw size={12} />{/if}
				{checking ? 'checking' : 'Re-JIT'}
			</button>
			<span class="tabular timing" aria-live="polite">
				{jitMs === null ? '—.———— ms' : `${jitMs.toFixed(2)} ms`}
			</span>
		</div>
	</div>

	<div class="workbench-grid">
		<div class="pane source-pane">
			<p class="pane-label">source (click to edit)</p>
			<div class="flex">
				<div class="gutter" aria-hidden="true">
					{#each lines as _, i}
						<div>{i + 1}</div>
					{/each}
				</div>
				<div class="relative min-w-0 flex-1">
					<pre class="highlight" aria-hidden="true">{#each lines as line}<div>{@html highlight(line) || ' '}</div>{/each}</pre>
					<textarea
						bind:value={source}
						oninput={onEdit}
						onkeydown={onTab}
						onscroll={(e) => {
							const pre = (e.target as HTMLElement).previousElementSibling as HTMLElement;
							if (pre) {
								pre.scrollTop = (e.target as HTMLElement).scrollTop;
								pre.scrollLeft = (e.target as HTMLElement).scrollLeft;
							}
						}}
						spellcheck={false}
						aria-label="Editable Rasmalai source"
						class="editor"
					></textarea>
				</div>
			</div>
		</div>

		<div class="pane output-pane">
			<p class="pane-label">compiler output (real check, off-thread)</p>
			<pre class="output" class:text-aura-muted={diag.startsWith('check ok') || diag.startsWith('Hit Re-JIT')}>{diag}</pre>
		</div>
	</div>
</div>

<style>
	.workbench {
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		background: var(--color-paper-2);
		overflow: hidden;
	}

	.workbench-head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--space-xs) var(--space-md);
		border-bottom: 1px solid var(--color-rule);
		padding: var(--space-xs) var(--space-md);
	}

	.file {
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		color: var(--color-ink);
	}

	.hint {
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.run {
		display: inline-flex;
		height: 30px;
		align-items: center;
		gap: 6px;
		border-radius: var(--radius-xs);
		background: var(--color-accent);
		padding: 0 var(--space-sm);
		font-size: var(--text-xs);
		font-weight: 600;
		color: var(--color-paper);
	}

	.run:disabled {
		cursor: not-allowed;
		background: rgb(var(--aura-purple) / 0.35);
		color: rgb(var(--aura-bg) / 0.65);
	}

	.timing {
		min-width: 88px;
		text-align: right;
		font-size: var(--text-xs);
		color: var(--color-signal);
	}

	.workbench-grid {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
	}

	.pane {
		min-width: 0;
	}

	.source-pane {
		border-right: 1px solid var(--color-rule);
	}

	.pane-label {
		border-bottom: 1px solid var(--color-rule);
		padding: var(--space-xs) var(--space-md);
		font-family: var(--font-body);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.gutter {
		user-select: none;
		border-right: 1px solid var(--color-rule);
		padding: var(--space-sm) var(--space-xs) var(--space-sm) var(--space-sm);
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		line-height: 1.85;
		text-align: right;
		color: rgb(var(--aura-text) / 0.22);
	}

	.highlight {
		margin: 0;
		overflow-x: auto;
		padding: var(--space-sm) var(--space-md);
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		line-height: 1.85;
		white-space: pre;
		color: var(--color-ink);
	}

	.editor {
		position: absolute;
		inset: 0;
		margin: 0;
		resize: none;
		overflow: auto;
		background: transparent;
		padding: var(--space-sm) var(--space-md);
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		line-height: 1.85;
		white-space: pre;
		color: transparent;
		caret-color: var(--color-accent);
		outline: none;
		scrollbar-width: none;
	}

	.output-pane {
		display: flex;
		min-height: 240px;
		flex-direction: column;
	}

	.output {
		margin: 0;
		min-height: 0;
		flex: 1;
		overflow: auto;
		padding: var(--space-md);
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		line-height: 1.8;
		white-space: pre-wrap;
		color: var(--color-ink);
	}

	@media (max-width: 860px) {
		.workbench-grid {
			grid-template-columns: minmax(0, 1fr);
		}
		.source-pane {
			border-right: 0;
			border-bottom: 1px solid var(--color-rule);
		}
	}
</style>
