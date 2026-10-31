<script lang="ts">
	import { withHost } from '$lib/docs/host';
	import { canonicalUrl } from '$lib/site';
	import { ETC_SECTIONS } from '$lib/docs/nav';
	import DocArticle from '$lib/components/DocArticle.svelte';

	let { data } = $props();

	const meta = $derived(ETC_SECTIONS.find((m) => m.slug === data.slug));
	let html = $derived(withHost(data.html));
</script>

<svelte:head>
	<title>{data.title} | Rasmalai Registry &amp; Policy</title>
	<meta name="description" content={data.description} />
	{@html `<script type="application/ld+json">${JSON.stringify({
		'@context': 'https://schema.org',
		'@type': 'TechArticle',
		headline: data.title,
		description: data.description,
		url: canonicalUrl(`/etc/${data.slug}`),
		inLanguage: 'en',
		author: { '@type': 'Organization', name: 'Rasmalai' }
	})}<\/script>`}
</svelte:head>

<DocArticle
	title={data.title}
	{html}
	raw={data.raw}
	sections={data.sections}
	prev={data.prev}
	next={data.next}
	icon={meta?.icon}
	tone="accent"
/>
