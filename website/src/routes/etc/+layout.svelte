<script lang="ts">
	import { ETC_SECTIONS } from '$lib/docs/nav';
	import DocsShell from '$lib/components/DocsShell.svelte';
	import { page } from '$app/state';

	let { children } = $props();
</script>

{#snippet nav()}
	<p class="docs-group">Registry &amp; policy</p>
	<ul class="docs-list">
		<li>
			<a href="/etc" aria-current={page.url.pathname === '/etc' ? 'page' : undefined}>
				Overview
			</a>
		</li>
		{#each ETC_SECTIONS as s}
			<li>
				<a
					href={s.path}
					aria-current={page.url.pathname === s.path ? 'page' : undefined}
					class="docs-icon"
				>
					<s.icon size={13} strokeWidth={1.75} class="shrink-0" />
					{s.title}
				</a>
			</li>
		{/each}
	</ul>

	<p class="docs-group">Elsewhere</p>
	<ul class="docs-list">
		<li><a href="/packages">Package registry</a></li>
		<li><a href="/manual">Language manual</a></li>
		<li><a href="/license">License</a></li>
	</ul>
{/snippet}

{#snippet header()}
	<nav class="docs-crumb" aria-label="Breadcrumb">
		{#if page.url.pathname === '/etc'}
			<span class="here">Registry</span>
		{:else}
			<a href="/etc">Registry</a>
		{/if}
	</nav>
{/snippet}

<DocsShell label="Registry" drawerLabel="Registry" toc={page.data.toc ?? []} {nav} {header}>
	{@render children()}
</DocsShell>
