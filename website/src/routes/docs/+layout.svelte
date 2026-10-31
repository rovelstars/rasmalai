<script lang="ts">
	import { MANUAL_SECTIONS, STDLIB, TRACKS, ROSETTA } from '$lib/docs/nav';
	import DialectLens from '$lib/components/DialectLens.svelte';
	import DocsShell from '$lib/components/DocsShell.svelte';
	import { page } from '$app/state';

	let { children } = $props();
</script>

{#snippet nav()}
	<p class="docs-group">Tracks</p>
	<ul class="docs-list">
		<li>
			<a href="/docs" aria-current={page.url.pathname === '/docs' ? 'page' : undefined}>Overview</a>
		</li>
		{#each TRACKS as t}
			{@const href = `/docs/tracks/${t.slug}`}
			<li><a href={href} aria-current={page.url.pathname === href ? 'page' : undefined}>{t.title}</a></li>
		{/each}
	</ul>

	<p class="docs-group">Coming from</p>
	<ul class="docs-list">
		{#each ROSETTA as r}
			{@const href = `/docs/coming-from/${r.slug}`}
			<li><a href={href} aria-current={page.url.pathname === href ? 'page' : undefined}>{r.title}</a></li>
		{/each}
	</ul>

	<p class="docs-group">Language manual</p>
	<ul class="docs-list">
		{#each MANUAL_SECTIONS as m}
			<li><a href={m.path} aria-current={page.url.pathname === m.path ? 'page' : undefined}>{m.title}</a></li>
		{/each}
	</ul>

	<p class="docs-group">Standard library</p>
	<ul class="docs-list">
		{#each STDLIB as m}
			{@const href = `/docs/@std/${m}/overview`}
			<li><a href={href} aria-current={page.url.pathname === href ? 'page' : undefined}>{m}</a></li>
		{/each}
	</ul>
{/snippet}

{#snippet header()}
	<div class="flex items-center justify-between gap-4">
		<nav class="docs-crumb" aria-label="Breadcrumb">
			{#if page.url.pathname === '/docs'}
				<span class="here">Docs</span>
			{:else}
				<a href="/docs">Docs</a>
			{/if}
		</nav>
		<div class="hidden shrink-0 sm:block">
			<DialectLens compact />
		</div>
	</div>
{/snippet}

{#if /^\/docs\/[^/]+\/[^/]+/.test(page.url.pathname)}
	{@render children()}
{:else}
	<DocsShell label="Docs" drawerLabel="Docs" toc={page.data.toc ?? []} {nav} {header}>
		{@render children()}
	</DocsShell>
{/if}
