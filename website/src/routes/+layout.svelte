<script lang="ts">
	import '@fontsource/inter/400.css';
	import '@fontsource/inter/600.css';
	import '@fontsource/inter/700.css';
	import '@fontsource/jetbrains-mono/400.css';
	import '@fontsource/jetbrains-mono/600.css';
	import '../app.css';
	import '../harness.css';
	import { onMount } from 'svelte';
	import { afterNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import Navbar from '$lib/components/Navbar.svelte';
	import { enhanceDocCode } from '$lib/docs/enhance';
	import { ogImageFor } from '$lib/docs/og';
	import ogManifest from '../../static/og/manifest.json';

	let { children } = $props();

	const canonical = $derived(`https://rasmalai.rovelstars.com${page.url.pathname}`);
	const ogImage = $derived(`https://rasmalai.rovelstars.com${ogImageFor(page.url.pathname, ogManifest)}`);
	const ogUrl = $derived.by(() => {
		if (!page.url.pathname.startsWith('/packages/')) return canonical;
		const segs = page.url.pathname.split('/').filter(Boolean).slice(1);
		if (segs.length > 1 && /^\d+\.\d+\.\d+$/.test(segs[segs.length - 1])) segs.pop();
		return `https://rasmalai.rovelstars.com/packages/${segs.join('/')}`;
	});

	function enhance() {
		try {
			enhanceDocCode(document.body);
		} catch {
			/* enhancement is best-effort */
		}
	}

	onMount(() => {
		enhance();
	});

	afterNavigate(() => {
		enhance();
	});
</script>

<svelte:head>
	<title>Rasmalai — Memory safety without the negotiations.</title>
	<meta
		name="description"
		content="A systems language with deterministic memory reclamation, Cranelift dev builds, and zero-codegen semantic checks."
	/>
	{#if page.url.pathname.startsWith('/packages/')}
		<!-- Package pages emit their own version-aware canonical. -->
	{:else}
		<link rel="canonical" href={canonical} />
	{/if}
	<meta property="og:type" content="website" />
	<meta property="og:site_name" content="Rasmalai" />
	<meta property="og:title" content="Rasmalai — Memory safety without the negotiations." />
	<meta
		property="og:description"
		content="A systems language with deterministic memory reclamation, Cranelift dev builds, and zero-codegen semantic checks."
	/>
	<meta property="og:url" content={ogUrl} />
	<meta property="og:image" content={ogImage} />
	<meta property="og:image:width" content="1200" />
	<meta property="og:image:height" content="630" />
	<meta property="og:image:alt" content="Rasmalai — memory safety without the negotiations." />
	<meta name="twitter:card" content="summary_large_image" />
	<meta name="twitter:title" content="Rasmalai — Memory safety without the negotiations." />
	<meta
		name="twitter:description"
		content="A systems language with deterministic memory reclamation, Cranelift dev builds, and zero-codegen semantic checks."
	/>
	<meta name="twitter:image" content={ogImage} />
	<meta name="theme-color" content="#14131a" />
</svelte:head>

<Navbar />
{@render children()}
