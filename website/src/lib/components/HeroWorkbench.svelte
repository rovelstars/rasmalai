<script lang="ts">
	import { LoaderCircle } from 'lucide-svelte';
	import { engine } from '$lib/playground/engine.svelte';

	let source = $state(
		`import { Vec4f } from "@std/simd";

fn shade(sun: Vec4f): Float {
    defer { print("shade done"); }
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
				if (com) return `<span class="text-aura-muted italic">${m}</span>`;
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

<div class="bay" data-testid="hero-workbench">
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-aura-border px-4 py-2 font-mono text-[11px]">
		<span class="text-aura-muted">src/shade.rnx</span>
		<span class="hidden text-aura-muted/60 sm:inline">[ on-demand check ]</span>
		<div class="ml-auto flex items-center gap-2">
			<button
				onclick={checkNow}
				disabled={checking}
				class="press flex h-7 min-w-[76px] items-center justify-center gap-1.5 rounded bg-aura-purple px-3 py-1 font-semibold text-aura-bg disabled:opacity-50"
			>
				{#if checking}<LoaderCircle size={12} class="animate-spin" />{:else}Re-JIT{/if}
			</button>
			<span class="tabular min-w-20 text-right text-aura-green" aria-live="polite">
				{jitMs === null ? '—.———— ms' : `${jitMs.toFixed(2)} ms`}
			</span>
		</div>
	</div>

	<div class="grid lg:grid-cols-2">
		<div class="border-b border-aura-border lg:border-r lg:border-b-0">
			<p class="border-b border-aura-border px-4 py-1.5 font-mono text-[11px] text-aura-muted">
				+ source — click to edit
			</p>
			<div class="flex">
				<div class="select-none border-r border-aura-border py-3 pr-2 pl-3 font-mono text-[12px] leading-6" aria-hidden="true">
					{#each lines as _, i}
						<div class="px-1 text-right text-aura-text/20">{i + 1}</div>
					{/each}
				</div>
				<div class="relative min-w-0 flex-1">
					<pre
						class="pointer-events-none m-0 overflow-x-auto px-3 py-3 font-mono text-[12px] leading-6 whitespace-pre text-aura-text"
						aria-hidden="true">{#each lines as line}<div>{@html highlight(line) || ' '}</div>{/each}</pre>
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
						class="absolute inset-0 m-0 resize-none overflow-auto bg-transparent px-3 py-3 font-mono text-[12px] leading-6 whitespace-pre text-transparent caret-aura-purple outline-none"
					></textarea>
				</div>
			</div>
		</div>

		<div class="flex h-[240px] min-w-0 flex-col">
			<p class="border-b border-aura-border px-4 py-1.5 font-mono text-[11px] text-aura-muted">
				+ compiler output — real check, off-thread
			</p>
			<pre class="tabular m-0 min-h-0 flex-1 overflow-auto p-4 font-mono text-[12px] leading-relaxed whitespace-pre-wrap {diag.startsWith('check ok') || diag.startsWith('Hit Re-JIT to download') ? 'text-aura-muted' : 'text-aura-text'}">{diag}</pre>
		</div>
	</div>
</div>

<style>
	textarea {
		scrollbar-width: none;
	}
</style>
