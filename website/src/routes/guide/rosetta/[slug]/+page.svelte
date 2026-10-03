<script lang="ts">
	import { onMount } from 'svelte';
	import RosettaCode from '$lib/components/RosettaCode.svelte';
	import LangIcon from '$lib/components/LangIcon.svelte';
	import { setDialect, type Dialect } from '$lib/stores/dialect.svelte';
	import { ROSETTA } from '$lib/docs/nav';
	import { withHost } from '$lib/docs/host';
	import { onDocCodeClick } from '$lib/docs/codeActions';

	let { data } = $props();

	const slug = $derived(data.slug.replace(/^from-/, ''));
	const meta = $derived(ROSETTA.find((r) => r.slug === slug));
	let sections = $derived(data.sections.map((s) => ({ ...s, html: withHost(s.html) })));

	onMount(() => {
		const lang = data.sourceLang.toLowerCase();
		if (lang === 'rust' || lang === 'go' || lang === 'cpp' || lang === 'typescript') {
			setDialect(lang as Dialect);
		}
	});
</script>

<svelte:head>
	<title>{data.title} — Rasmalai Guide</title>
	<meta name="description" content={data.description} />
</svelte:head>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events: delegated code-block handler -->
<article class="doc-prose" onclick={onDocCodeClick}>
	<nav class="mb-4 font-mono text-xs text-aura-muted" aria-label="Breadcrumb">
		<a href="/guide" class="hover:text-aura-text">Guide</a>
		<span class="mx-1.5" aria-hidden="true">/</span>
		<a href="/guide#rosetta" class="hover:text-aura-text">Rosetta</a>
		<span class="mx-1.5" aria-hidden="true">/</span>
		<span class="text-aura-text">{data.title}</span>
	</nav>
	<h1 class="flex items-center gap-3">
		{#if meta}<LangIcon lang={meta.dialect} size={26} />{/if}
		{data.title}
	</h1>
	{#each sections as s}
		{@html s.html}
		{#if s.compare}
			<RosettaCode rnx={s.compare.rnx} lang={s.compare.lang} other={s.compare.other} />
		{/if}
	{/each}
</article>
