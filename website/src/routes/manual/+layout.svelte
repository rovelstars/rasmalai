<script lang="ts">
	import { MANUAL_SECTIONS, MANUAL_PARTS } from '$lib/docs/nav';
	import { BookOpen } from 'lucide-svelte';
	import DocsShell from '$lib/components/DocsShell.svelte';
	import { page } from '$app/state';

	let { children } = $props();
</script>

{#snippet nav()}
	<p class="docs-group">Language manual</p>
	<ul class="docs-list">
		<li>
			<a
				href="/manual"
				aria-current={page.url.pathname === '/manual' ? 'page' : undefined}
				class="docs-icon"
			>
				<BookOpen size={13} strokeWidth={1.75} />
				Overview
			</a>
		</li>
	</ul>

	{#each MANUAL_PARTS as part, i}
		<p class="docs-group">Part {i + 1} - {part}</p>
		<ul class="docs-list">
			{#each MANUAL_SECTIONS.filter((m) => m.part === part) as m}
				{@const active = page.url.pathname === m.path}
				<li>
					<a href={m.path} aria-current={active ? 'page' : undefined} class="docs-icon">
						<m.icon size={13} strokeWidth={1.75} class="shrink-0" />
						{m.title}
					</a>
				</li>
			{/each}
		</ul>
	{/each}

	<p class="docs-group">Learn</p>
	<ul class="docs-list">
		<li><a href="/guide">Guide</a></li>
		<li><a href="/playground">Playground</a></li>
	</ul>
{/snippet}

{#snippet header()}
	<nav class="docs-crumb" aria-label="Breadcrumb">
		{#if page.url.pathname === '/manual'}
			<span class="here">Manual</span>
		{:else}
			<a href="/manual">Manual</a>
		{/if}
	</nav>
{/snippet}

<DocsShell label="Manual" drawerLabel="Manual" toc={page.data.toc ?? []} {nav} {header}>
	{@render children()}
</DocsShell>
