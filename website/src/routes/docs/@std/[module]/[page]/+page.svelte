<script lang="ts">
	import ModuleDocs from '$lib/components/ModuleDocs.svelte';
	import { withHost } from '$lib/docs/host';
	import { onDocCodeClick } from '$lib/docs/codeActions';
	import { attachSectionCopy, onSectionCopyClick } from '$lib/docs/sectionCopy';
	import { stdMeta } from '$lib/docs/stdlib';

	let { data } = $props();

	const titles: Record<string, string> = {
		overview: 'Overview',
		'getting-started': 'Getting Started',
		api: 'API Reference'
	};

	let html = $derived(withHost(data.html ?? ''));
	let articleEl: HTMLElement | undefined = $state();
	const pageRaw = $derived('raw' in data ? (data.raw as string) : null);
	const pageSections = $derived('sections' in data ? data.sections : null);

	$effect(() => {
		if (articleEl) attachSectionCopy(articleEl, pageRaw, pageSections);
	});
</script>

<svelte:head>
	<title>@std/{data.module} - {titles[data.page] ?? data.page} — Rasmalai Docs</title>
	<meta name="description" content={stdMeta(data.module).tagline} />
	{@html `<script type="application/ld+json">${JSON.stringify({
		'@context': 'https://schema.org',
		'@type': 'TechArticle',
		headline: `@std/${data.module} - ${titles[data.page] ?? data.page}`,
		description: stdMeta(data.module).tagline,
		url: `https://rnx.dev/docs/@std/${data.module}/${data.page}`,
		inLanguage: 'en',
		author: { '@type': 'Organization', name: 'Rasmalai' }
	})}<\/script>`}
</svelte:head>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events: delegated code-block handler -->
<article class="doc-prose" bind:this={articleEl} onclick={(e) => { onDocCodeClick(e); onSectionCopyClick(e); }}>
	<div class="flex items-start justify-between gap-3">
		<h1 class="flex items-center gap-3">
			{titles[data.page] ?? data.page}
		</h1>
		{#if data.html}
			<button type="button" class="doc-btn mt-1 shrink-0" data-copy-page title="Copy full page markdown">copy page</button>
		{/if}
	</div>
	{#if data.module === 'prelude'}
		<p class="mt-3 rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm">
			<span class="font-mono text-[11px] uppercase tracking-wider text-emerald-400">implicit scope</span><br />
			Symbols in <code class="font-mono text-[13px]">@std/prelude</code> are available in every Rasmalai
			source file out-of-the-box without an explicit import. Explicit imports are supported for
			disambiguation.
		</p>
	{/if}
	{#if data.noApi}
		<div class="panel mt-6 p-6">
			<h2 class="text-lg font-bold">No API data yet</h2>
			<p class="mt-2 text-sm text-aura-muted">
				The documented snapshot for <code class="font-mono text-[13px]">@std/{data.module}</code>
				publishes with the next release. The module itself ships with the compiler —
				only this reference page is waiting on data.
			</p>
		</div>
	{:else if data.html}
		<!-- eslint-disable-next-line svelte/no-at-html-tags -->
		{@html html}
	{:else if data.mod}
		<ModuleDocs mod={data.mod} />
	{/if}
</article>
