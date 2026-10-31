<script lang="ts">
	import RosettaCode from '$lib/components/RosettaCode.svelte';
	import { withHost } from '$lib/docs/host';
	import { onDocCodeClick } from '$lib/docs/codeActions';
	import { TRACKS } from '$lib/docs/nav';

	let { data } = $props();

	const meta = $derived(TRACKS.find((t) => t.slug === data.slug));
	let sections = $derived(data.sections.map((s) => ({ ...s, html: withHost(s.html) })));

</script>

<svelte:head>
	<title>{data.title} | Rasmalai Docs</title>
	<meta name="description" content={data.description} />
</svelte:head>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events: delegated code-block handler -->
<article class="doc-prose" onclick={onDocCodeClick}>
	<h1 class="doc-h1">
		{#if meta}
			<meta.icon size={22} strokeWidth={2} class="doc-h1-icon is-signal" />
		{/if}
		{data.title}
	</h1>
	{#each sections as s}
		{@html s.html}
		{#if s.compare}
			<RosettaCode rnx={s.compare.rnx} lang={s.compare.lang} other={s.compare.other} />
		{/if}
	{/each}
</article>
