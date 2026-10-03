<script lang="ts">
	import type { TocEntry } from '$lib/docs/markdown';
	import { GUIDE_CHAPTERS, TRACKS, ROSETTA } from '$lib/docs/nav';
	import { ArrowRight, BookOpen, X, Diamond } from 'lucide-svelte';
	import LangIcon from '$lib/components/LangIcon.svelte';
	import { page } from '$app/state';

	let { children } = $props();
	let toc: TocEntry[] = $derived(page.data.toc ?? []);
	let activeId = $state('');
	let drawer = $state(false);

	$effect(() => {
		void page.url.pathname;
		activeId = '';
		drawer = false;
		const root = document.querySelector('[data-docs-content]');
		if (!root) return;
		const targets = root.querySelectorAll('[id]');
		if (targets.length === 0) return;
		const obs = new IntersectionObserver(
			(entries) => {
				for (const e of entries) {
					if (e.isIntersecting) activeId = (e.target as HTMLElement).id;
				}
			},
			{ rootMargin: '-20% 0px -70% 0px' }
		);
		targets.forEach((t) => obs.observe(t));
		return () => obs.disconnect();
	});
</script>

{#snippet navtree()}
	<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">the book</p>
	<ul class="mt-2 space-y-1">
		<li>
			<a
				href="/guide"
				class="flex items-center gap-2 rounded px-2 py-1 {page.url.pathname === '/guide'
					? 'bg-aura-surfaceElevated text-aura-text'
					: 'text-aura-muted hover:text-aura-text'}"
			>
				<BookOpen size={14} strokeWidth={1.75} class="shrink-0 {page.url.pathname === '/guide' ? 'text-aura-purple' : ''}" />
				Overview
			</a>
		</li>
	</ul>
	<p class="mt-6 font-mono text-[11px] uppercase tracking-wider text-aura-muted">guide</p>
	<ul class="mt-2 space-y-1">
		{#each GUIDE_CHAPTERS as c}
			{@const active = page.url.pathname === c.path}
			<li>
				<a
					href={c.path}
					class="flex items-center gap-2 rounded px-2 py-1 {active
						? 'bg-aura-surfaceElevated text-aura-text'
						: 'text-aura-muted hover:text-aura-text'}"
				>
					<c.icon size={14} strokeWidth={1.75} class="shrink-0 {active ? 'text-aura-green' : ''}" />
					{c.title}
				</a>
			</li>
		{/each}
	</ul>
	<p class="mt-6 font-mono text-[11px] uppercase tracking-wider text-aura-muted">tracks</p>
	<ul class="mt-2 space-y-1">
		{#each TRACKS as t}
			{@const active = page.url.pathname === `/guide/tracks/${t.slug}`}
			<li>
				<a
					href="/guide/tracks/{t.slug}"
					class="flex items-center gap-2 rounded px-2 py-1 {active
						? 'bg-aura-surfaceElevated text-aura-text'
						: 'text-aura-muted hover:text-aura-text'}"
				>
					<t.icon size={14} strokeWidth={1.75} class="shrink-0 {active ? 'text-aura-green' : ''}" />
					{t.title}
				</a>
			</li>
		{/each}
	</ul>
	<p class="mt-6 font-mono text-[11px] uppercase tracking-wider text-aura-muted">coming from</p>
	<ul class="mt-2 space-y-1">
		{#each ROSETTA as r}
			{@const active = page.url.pathname === r.path}
			<li>
				<a
					href={r.path}
					class="flex items-center gap-2 rounded px-2 py-1 {active
						? 'bg-aura-surfaceElevated text-aura-text'
						: 'text-aura-muted hover:text-aura-text'}"
				>
					<LangIcon lang={r.dialect} size={14} />
					{r.title}
				</a>
			</li>
		{/each}
	</ul>
	<p class="mt-6 font-mono text-[11px] uppercase tracking-wider text-aura-muted">manual</p>
	<ul class="mt-2 space-y-1">
		<li>
			<a
				href="/manual"
				class="flex items-center gap-2 rounded px-2 py-1 text-aura-muted hover:text-aura-text"
			>
				<BookOpen size={14} strokeWidth={1.75} class="shrink-0" />
				Language Manual <ArrowRight size={13} class="inline" />
			</a>
		</li>
	</ul>
{/snippet}

<div class="mx-auto max-w-7xl px-4 pb-16">
	<div class="flex items-center gap-3 pt-6">
		<button
			onclick={() => (drawer = true)}
			class="press flex min-h-8 min-w-8 items-center justify-center rounded-md border border-aura-border text-aura-muted hover:text-aura-text lg:hidden"
			aria-label="Open guide index"
		>
			<svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
				<path d="M2 4.5h14M2 9h14M2 13.5h14" stroke-linecap="round" />
			</svg>
		</button>
	</div>
	<div class="grid gap-8 pt-6 lg:grid-cols-[220px_minmax(0,1fr)_200px]">
		<nav class="hidden lg:block" aria-label="Guide">
			<div class="sticky top-24 max-h-[calc(100vh-7rem)] overflow-auto text-sm">
				{@render navtree()}
			</div>
		</nav>

		<div class="min-w-0" data-docs-content>
			{@render children()}
		</div>

		<aside class="hidden lg:block" aria-label="On this page">
			<div class="sticky top-24 max-h-[calc(100vh-7rem)] overflow-auto text-[13px]">
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">on this page</p>
				<ul class="mt-2 space-y-1">
					{#each toc as t}
						<li class={t.level === 3 ? 'pl-3' : ''}>
							<a
								href="#{t.id}"
								class="block py-0.5 {activeId === t.id
									? 'text-aura-cyan'
									: 'text-aura-muted hover:text-aura-cyan'}"
							>
								{t.text}
							</a>
						</li>
					{/each}
					{#if toc.length === 0}
						<li class="py-0.5 text-aura-muted">—</li>
					{/if}
				</ul>
			</div>
		</aside>
	</div>
</div>

{#if drawer}
	<!-- svelte-ignore a11y_click_events_have_key_events: backdrop has an Escape key handler below -->
	<div
		class="fixed inset-0 z-50 lg:hidden"
		onclick={() => (drawer = false)}
		onkeydown={(e) => {
			if (e.key === 'Escape') drawer = false;
		}}
		role="presentation"
	>
		<div class="absolute inset-0 bg-black/60" aria-hidden="true"></div>
		<nav
			class="drawer-slide absolute bottom-0 left-0 top-0 w-72 overflow-auto border-r border-aura-border bg-aura-bg p-4 text-sm"
			aria-label="Guide index"
		>
			<div class="mb-3 flex items-center justify-between">
				<span class="flex items-center gap-1 font-mono text-xs text-aura-purple"><Diamond size={11} />the book</span>
				<button
					onclick={() => (drawer = false)}
					class="rounded border border-aura-border px-2 py-0.5 font-mono text-xs text-aura-muted"
					aria-label="Close guide index"
				>
					<X size={14} />
				</button>
			</div>
			{@render navtree()}
		</nav>
	</div>
{/if}
