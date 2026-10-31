<script lang="ts">
	import { GUIDE_CHAPTERS, TRACKS, ROSETTA } from '$lib/docs/nav';
	import { BookOpen } from 'lucide-svelte';
	import LangIcon from '$lib/components/LangIcon.svelte';
	import DocsShell from '$lib/components/DocsShell.svelte';
	import { page } from '$app/state';

	let { children } = $props();
</script>

{#snippet nav()}
	<p class="docs-group">The book</p>
	<ul class="docs-list">
		<li>
			<a
				href="/guide"
				aria-current={page.url.pathname === '/guide' ? 'page' : undefined}
				class="docs-icon"
			>
				<BookOpen size={13} strokeWidth={1.75} />
				Overview
			</a>
		</li>
	</ul>

	<p class="docs-group">Chapters</p>
	<ul class="docs-list">
		{#each GUIDE_CHAPTERS as c}
			{@const active = page.url.pathname === c.path}
			<li>
				<a href={c.path} aria-current={active ? 'page' : undefined} class="docs-icon">
					<c.icon size={13} strokeWidth={1.75} class="shrink-0" />
					{c.title}
				</a>
			</li>
		{/each}
	</ul>

	<p class="docs-group">Tracks</p>
	<ul class="docs-list">
		{#each TRACKS as t}
			{@const href = `/guide/tracks/${t.slug}`}
			<li>
				<a href={href} aria-current={page.url.pathname === href ? 'page' : undefined} class="docs-icon">
					<t.icon size={13} strokeWidth={1.75} class="shrink-0" />
					{t.title}
				</a>
			</li>
		{/each}
	</ul>

	<p class="docs-group">Coming from</p>
	<ul class="docs-list">
		{#each ROSETTA as r}
			<li>
				<a href={r.path} aria-current={page.url.pathname === r.path ? 'page' : undefined} class="docs-icon">
					<LangIcon lang={r.dialect} size={13} />
					{r.title}
				</a>
			</li>
		{/each}
	</ul>

	<p class="docs-group">Reference</p>
	<ul class="docs-list">
		<li><a href="/manual">Language manual</a></li>
		<li><a href="/packages">Package registry</a></li>
	</ul>
{/snippet}

{#snippet header()}
	<nav class="docs-crumb" aria-label="Breadcrumb">
		{#if page.url.pathname === '/guide'}
			<span class="here">Guide</span>
		{:else}
			<a href="/guide">Guide</a>
		{/if}
	</nav>
{/snippet}

<DocsShell label="Guide" drawerLabel="Guide" toc={page.data.toc ?? []} {nav} {header}>
	{@render children()}
</DocsShell>
