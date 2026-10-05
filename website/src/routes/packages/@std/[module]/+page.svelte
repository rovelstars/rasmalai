<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { browser } from '$app/environment';
	import { BookOpen } from 'lucide-svelte';
	import { STDLIB_ICONS } from '$lib/docs/nav';
	import { STD_LICENSE, firstParagraph, importSnippet } from '$lib/docs/stdlib';
	import { renderMarkdown } from '$lib/docs/markdown';

	let { data } = $props();
	let copied = $state(false);

	let version = $derived.by(() => {
		if (!browser) return data.engine;
		return page.url.searchParams.get('v') ?? data.engine;
	});
	let pinned = $derived(version === data.engine);

	let Icon = $derived(STDLIB_ICONS[data.meta.name]);
	let blurb = $derived(firstParagraph(data.mod) ?? data.meta.tagline);
	let blurbHtml = $derived.by(() => {
		try {
			return renderMarkdown(firstParagraph(data.mod) ?? data.meta.tagline).html;
		} catch {
			return null;
		}
	});
	let plainBlurb = $derived(blurb.replace(/[`*_]/g, ''));
	let counts = $derived.by(() => {
		if (!data.mod) return null;
		const parts = [
			`${data.mod.classes.length} classes`,
			`${data.mod.functions.length} functions`,
			`${data.mod.enums.length} enums`
		];
		if (data.mod.constants.length > 0) parts.push(`${data.mod.constants.length} constants`);
		return parts.join(', ');
	});

	async function copyImport() {
		try {
			await navigator.clipboard.writeText(importSnippet(data.meta.name));
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			copied = false;
		}
	}

	function pickVersion(v: string) {
		if (!browser) return;
		const params = new URLSearchParams(page.url.searchParams);
		params.set('v', v);
		goto(`?${params.toString()}`, { keepFocus: true });
	}
</script>

<svelte:head>
	<title>@std/{data.meta.name} — Packages</title>
	<meta name="description" content={plainBlurb} />
	{@html `<script type="application/ld+json">${JSON.stringify({
		'@context': 'https://schema.org',
		'@type': 'SoftwareApplication',
		name: `@std/${data.meta.name}`,
		description: plainBlurb,
		url: `https://rnx.dev/packages/@std/${data.meta.name}`,
		applicationCategory: 'DeveloperApplication',
		operatingSystem: 'Linux, macOS, Windows',
		offers: { '@type': 'Offer', price: '0' }
	})}<\/script>`}
</svelte:head>

<main class="mx-auto max-w-7xl px-4 pb-16">
	<nav class="mt-8 font-mono text-xs text-aura-muted" aria-label="Breadcrumb">
		<a href="/packages" class="hover:text-aura-text">Packages</a>
		<span class="mx-1.5" aria-hidden="true">/</span>
		<span class="text-aura-text">@std/{data.meta.name}</span>
	</nav>

	<div class="panel mt-4 p-5">
		<div class="flex flex-wrap items-center gap-x-3 gap-y-2">
			{#if Icon}<Icon size={22} strokeWidth={1.75} class="shrink-0 text-aura-orange" />{/if}
			<span class="font-mono text-lg font-bold">@std/{data.meta.name}</span>
			<span class="rounded bg-aura-surfaceElevated px-1.5 py-0.5 font-mono text-[11px] text-aura-green"
				>standard library</span
			>
			<select
				value={version}
				onchange={(e) => pickVersion((e.target as HTMLSelectElement).value)}
				class="rounded border border-aura-border bg-aura-surfaceElevated px-2 py-0.5 font-mono text-xs"
				aria-label="Select version"
			>
				<option value={data.engine}>v{data.engine}</option>
				{#if !pinned}
					<option value={version}>v{version}</option>
				{/if}
			</select>
			<span class="rounded border border-aura-border px-1.5 py-0.5 font-mono text-[11px] text-aura-muted"
				>{STD_LICENSE}</span
			>
			<span class="tabular ml-auto font-mono text-xs text-aura-muted">engine >= {data.engine}</span>
		</div>
		{#if data.meta.name === 'prelude'}
			<p class="mt-3 rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm">
				<span class="font-mono text-[11px] uppercase tracking-wider text-emerald-400">implicit scope</span><br />
				Symbols in <code class="font-mono text-[13px]">@std/prelude</code> are available in every Rasmalai
				source file out-of-the-box without <code class="font-mono text-[13px]">import &#123; ... &#125; from "@std/prelude"</code>.
				Explicit imports are supported for disambiguation.
			</p>
		{/if}
		{#if !pinned}
			<p class="mt-2 rounded border border-aura-border px-3 py-2 text-sm text-aura-muted">
				Showing metadata for v{version}, but the standard library ships with the
				compiler — the documented snapshot below tracks engine v{data.engine}.
			</p>
		{/if}
		{#if blurbHtml}
			<div class="doc-prose mt-3 text-sm text-aura-muted">{@html blurbHtml}</div>
		{:else}
			<p class="mt-3 text-sm text-aura-muted">{blurb}</p>
		{/if}
		<div class="mt-3 flex items-center gap-2">
			<code class="tabular min-w-0 flex-1 overflow-x-auto rounded bg-aura-bg px-3 py-2 font-mono text-[13px] text-aura-cyan"
				>{importSnippet(data.meta.name)}</code
			>
			<button
				onclick={copyImport}
				class="press shrink-0 rounded border border-aura-border px-3 py-2 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
			>
				{copied ? 'copied' : 'copy'}
			</button>
		</div>
		<p class="tabular mt-3 font-mono text-[11px] text-aura-muted">
			{counts ?? 'No API data yet'}
		</p>
		<div class="mt-3">
			<a
				href="/docs/@std/{data.meta.name}/overview"
				class="press inline-flex items-center gap-2 rounded-md bg-aura-purple px-4 py-2 text-sm font-semibold text-[#15141b]"
			>
				<BookOpen size={15} strokeWidth={2} /> Read Documentation
			</a>
		</div>
	</div>

	<div class="mt-4 grid gap-4 md:grid-cols-2">
		<div class="panel p-4">
			<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">engine</p>
			<p class="mt-1 font-mono text-[13px]">rasmalai >= {data.engine}</p>
			<p class="mt-1 text-[13px] text-aura-muted">Ships inside the compiler. Nothing to install.</p>
		</div>
		<div class="panel p-4">
			<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">license</p>
			<p class="mt-1 text-sm">{STD_LICENSE} - Rasmalai contributors</p>
		</div>
	</div>
</main>
