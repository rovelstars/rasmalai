<script lang="ts">
	import type { TocEntry } from '$lib/docs/markdown';
	import { MANUAL_SECTIONS, STDLIB, TRACKS, ROSETTA } from '$lib/docs/nav';
	import DialectLens from '$lib/components/DialectLens.svelte';
	import { page } from '$app/state';

	let { children } = $props();
	let toc: TocEntry[] = $derived(page.data.toc ?? []);
	let activeId = $state('');

	$effect(() => {
		void page.url.pathname;
		activeId = '';
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

<div class="mx-auto max-w-7xl px-4 pb-16">
	{#if page.url.pathname.startsWith('/docs/@std/')}
		{@render children()}
	{:else}
	<div class="flex items-center justify-between gap-3 pt-6">
		<details class="lg:hidden">
			<summary class="cursor-pointer font-mono text-sm text-aura-muted">docs index</summary>
			<div class="mt-2 grid grid-cols-2 gap-1 text-sm">
				<a href="/docs" class="rounded px-2 py-1 text-aura-muted">Overview</a>
				{#each TRACKS as t}
					<a href="/docs/tracks/{t.slug}" class="rounded px-2 py-1 text-aura-muted">{t.title}</a>
				{/each}
				{#each ROSETTA as r}
					<a href="/docs/coming-from/{r.slug}" class="rounded px-2 py-1 text-aura-muted">{r.title}</a>
				{/each}
				{#each MANUAL_SECTIONS as m}
					<a href={m.path} class="rounded px-2 py-1 text-aura-muted">{m.title}</a>
				{/each}
				{#each STDLIB as m}
					<a href="/docs/@std/{m}/overview" class="rounded px-2 py-1 font-mono text-[13px] text-aura-muted"
						>@std/{m}</a
					>
				{/each}
			</div>
		</details>
		<div class="ml-auto hidden sm:block">
			<DialectLens compact />
		</div>
	</div>
	<div class="grid gap-8 pt-6 lg:grid-cols-[220px_minmax(0,1fr)_200px]">
		<nav class="hidden lg:block" aria-label="Docs">
			<div class="sticky top-24 max-h-[calc(100vh-7rem)] overflow-auto text-sm">
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">
					tracks & overview
				</p>
				<ul class="mt-2 space-y-1">
					<li>
						<a
							href="/docs"
							class="block rounded px-2 py-1 {page.url.pathname === '/docs'
								? 'bg-aura-surfaceElevated text-aura-text'
								: 'text-aura-muted hover:text-aura-text'}"
						>
							Overview
						</a>
					</li>
					{#each TRACKS as t}
						<li>
							<a
								href="/docs/tracks/{t.slug}"
								class="block rounded px-2 py-1 {page.url.pathname === `/docs/tracks/${t.slug}`
									? 'bg-aura-surfaceElevated text-aura-text'
									: 'text-aura-muted hover:text-aura-text'}"
							>
								{t.title}
							</a>
						</li>
					{/each}
				</ul>
				<p class="mt-6 font-mono text-[11px] uppercase tracking-wider text-aura-muted">
					coming from
				</p>
				<ul class="mt-2 space-y-1">
					{#each ROSETTA as r}
						<li>
							<a
								href="/docs/coming-from/{r.slug}"
								class="block rounded px-2 py-1 {page.url.pathname ===
								`/docs/coming-from/${r.slug}`
									? 'bg-aura-surfaceElevated text-aura-text'
									: 'text-aura-muted hover:text-aura-text'}"
							>
								{r.title}
							</a>
						</li>
					{/each}
				</ul>
				<p class="mt-6 font-mono text-[11px] uppercase tracking-wider text-aura-muted">
					language manual
				</p>
				<ul class="mt-2 space-y-1">
					{#each MANUAL_SECTIONS as m}
						<li>
							<a
								href={m.path}
								class="block rounded px-2 py-1 {page.url.pathname === m.path
									? 'bg-aura-surfaceElevated text-aura-text'
									: 'text-aura-muted hover:text-aura-text'}"
							>
								{m.title}
							</a>
						</li>
					{/each}
				</ul>
				<p class="mt-6 font-mono text-[11px] uppercase tracking-wider text-aura-muted">
					standard library
				</p>
				<ul class="mt-2 space-y-1">
					{#each STDLIB as m}
						<li>
							<a
								href="/docs/@std/{m}/overview"
								class="block rounded px-2 py-1 font-mono text-[13px] {page.url.pathname ===
								`/docs/@std/${m}/overview`
									? 'bg-aura-surfaceElevated text-aura-purple'
									: 'text-aura-muted hover:text-aura-text'}"
							>
								@std/{m}
							</a>
						</li>
					{/each}
				</ul>
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
	{/if}
</div>
