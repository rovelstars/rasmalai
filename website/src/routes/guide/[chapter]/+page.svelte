<script lang="ts">
	import { ArrowLeft, ArrowRight } from 'lucide-svelte';
	import { withHost } from '$lib/docs/host';
	import { onDocCodeClick } from '$lib/docs/codeActions';
	import { attachSectionCopy, onSectionCopyClick } from '$lib/docs/sectionCopy';
	import { GUIDE_CHAPTERS } from '$lib/docs/nav';

	let { data } = $props();

	const meta = $derived(GUIDE_CHAPTERS.find((c) => c.slug === data.slug));
	let html = $derived(withHost(data.html));
	let articleEl: HTMLElement | undefined = $state();

	$effect(() => {
		if (articleEl && data.slug) attachSectionCopy(articleEl, data.raw, data.sections);
	});
</script>

<svelte:head>
	<title>{data.title} — Rasmalai Guide</title>
	<meta name="description" content={data.description} />
	{@html `<script type="application/ld+json">${JSON.stringify({
		'@context': 'https://schema.org',
		'@type': 'TechArticle',
		headline: data.title,
		description: data.description,
		url: `https://rnx.dev/guide/${data.slug}`,
		inLanguage: 'en',
		author: { '@type': 'Organization', name: 'Rasmalai' }
	})}<\/script>`}
</svelte:head>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events: delegated code-block handler -->
<article class="doc-prose" bind:this={articleEl} onclick={(e) => { onDocCodeClick(e); onSectionCopyClick(e); }}>
	<nav class="mb-4 flex items-center justify-between gap-3 font-mono text-xs text-aura-muted" aria-label="Breadcrumb">
		<span>
			<a href="/guide" class="hover:text-aura-text">Guide</a>
			<span class="mx-1.5" aria-hidden="true">/</span>
			<span class="text-aura-text">{data.title}</span>
		</span>
		<button type="button" class="doc-btn" data-copy-page title="Copy full page markdown">copy page</button>
	</nav>
	<h1 class="flex items-center gap-3">
		{#if meta}<meta.icon size={26} strokeWidth={1.75} class="shrink-0 text-aura-green" />{/if}
		{data.title}
	</h1>
	<!-- eslint-disable-next-line svelte/no-at-html-tags -->
	{@html html}
	<div class="mt-10 flex items-center justify-between gap-4 border-t border-aura-border pt-5">
		{#if data.prev}
			<a href={data.prev.path} class="press panel block max-w-[45%] p-3 hover:border-aura-borderHover">
				<p class="inline-flex items-center gap-1 font-mono text-[11px] text-aura-muted"><ArrowLeft size={11} />previous</p>
				<p class="mt-0.5 text-sm font-semibold">{data.prev.title}</p>
			</a>
		{:else}
			<span></span>
		{/if}
		{#if data.next}
			<a href={data.next.path} class="press panel block max-w-[45%] p-3 text-right hover:border-aura-borderHover">
				<p class="inline-flex items-center gap-1 font-mono text-[11px] text-aura-muted">next<ArrowRight size={11} /></p>
				<p class="mt-0.5 text-sm font-semibold">{data.next.title}</p>
			</a>
		{/if}
	</div>
</article>
