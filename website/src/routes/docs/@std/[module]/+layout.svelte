<script lang="ts">
	import { goto } from '$app/navigation';
	import { ArrowLeft, X, Diamond } from 'lucide-svelte';
	import { page } from '$app/state';
	import { browser } from '$app/environment';
	import type { TocEntry } from '$lib/docs/markdown';
	import { STDLIB_ICONS } from '$lib/docs/nav';
	import { ENGINE_VERSION, stdMeta, stdApiModule, quickstartFor } from '$lib/docs/stdlib';
	import { encodeSnippet } from '$lib/playground/share';

	let { children } = $props();

	let modName = $derived(page.params.module ?? '');
	let meta = $derived.by(() => {
		try {
			return stdMeta(modName);
		} catch {
			return null;
		}
	});
	let Icon = $derived(modName ? STDLIB_ICONS[modName] : null);
	let toc: TocEntry[] = $derived(page.data.toc ?? []);
	let activeId = $state('');
	let drawer = $state(false);

	let symbols = $derived.by(() => {
		const mod = stdApiModule(modName);
		if (!mod) return [];
		return [
				...mod.classes.map((c) => ({ kind: 'Class', name: c.name, href: 'api' as const, anchor: `class-${c.name}` })),
				...mod.enums.map((e) => ({ kind: 'Enum', name: e.name, href: 'api' as const, anchor: `enum-${e.name}` })),
				...mod.functions.map((f) => ({ kind: 'Fn', name: f.name, href: 'api' as const, anchor: `fn-${f.name}` }))
			];
	});

	const SECTIONS = [
		{ slug: 'overview', title: 'Overview' },
		{ slug: 'getting-started', title: 'Getting Started' },
		{ slug: 'api', title: 'API Reference' }
	];

	let currentSection = $derived(page.params.page ?? 'overview');
	let tryCode = $derived.by(() => {
		try {
			return encodeSnippet(quickstartFor(modName));
		} catch {
			return null;
		}
	});
	let version = $derived.by(() => {
		if (!browser) return ENGINE_VERSION;
		return page.url.searchParams.get('v') ?? ENGINE_VERSION;
	});

	function pickVersion(v: string) {
		if (!browser) return;
		const params = new URLSearchParams(page.url.searchParams);
		params.set('v', v);
		goto(`?${params.toString()}`, { keepFocus: true });
	}

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
	<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">
		<a href="/packages/@std/{modName}" class="inline-flex items-center gap-1 hover:text-aura-text"><ArrowLeft size={12} />package</a>
	</p>
	<p class="mt-4 flex items-center gap-2 font-mono text-sm font-bold">
		{#if Icon}<Icon size={15} strokeWidth={1.75} class="shrink-0 text-aura-orange" />{/if}
		@std/{modName}
	</p>
	<div class="mt-2 flex items-center gap-2">
		<select
			value={version}
			onchange={(e) => pickVersion((e.target as HTMLSelectElement).value)}
			class="rounded border border-aura-border bg-aura-surface px-2 py-0.5 font-mono text-xs"
			aria-label="Select version"
		>
			<option value={ENGINE_VERSION}>v{ENGINE_VERSION}</option>
		</select>
		{#if tryCode}
			<a
				href="/playground?code={tryCode}"
				target="_blank"
				rel="noopener"
				class="press rounded border border-aura-border px-2 py-0.5 font-mono text-[11px] text-aura-green hover:border-aura-green"
			>
				try it &#9654;
			</a>
		{/if}
	</div>
	<ul class="mt-4 space-y-1">
		{#each SECTIONS as s}
			{@const href = `/docs/@std/${modName}/${s.slug}`}
			{@const active = currentSection === s.slug}
			<li>
				<a
					href={href}
					class="block rounded px-2 py-1 {active
						? 'bg-aura-surfaceElevated text-aura-text'
						: 'text-aura-muted hover:text-aura-text'}"
				>
					{s.title}
				</a>
			</li>
		{/each}
	</ul>
	{#if symbols.length > 0}
		<p class="mt-6 font-mono text-[11px] uppercase tracking-wider text-aura-muted">symbols</p>
		<ul class="mt-2 space-y-1 font-mono text-[13px]">
			{#each symbols as s}
				<li>
					<a
						href="/docs/@std/{modName}/{s.href}#{s.anchor}"
						class="block rounded px-2 py-0.5 text-aura-muted hover:text-aura-cyan"
					>
						<span class="text-aura-pink">{s.kind[0]}</span>
						<span class="ml-1">{s.name}</span>
					</a>
				</li>
			{/each}
		</ul>
	{/if}
{/snippet}

<div class="mx-auto max-w-7xl px-4 pb-16">
	<div class="flex items-center gap-3 pt-6">
		<button
			onclick={() => (drawer = true)}
			class="press flex min-h-8 min-w-8 items-center justify-center rounded-md border border-aura-border text-aura-muted hover:text-aura-text lg:hidden"
			aria-label="Open package index"
		>
			<svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
				<path d="M2 4.5h14M2 9h14M2 13.5h14" stroke-linecap="round" />
			</svg>
		</button>
		<nav class="ml-auto hidden font-mono text-xs text-aura-muted sm:block" aria-label="Breadcrumb">
			<a href="/packages" class="hover:text-aura-text">Packages</a>
			<span class="mx-1.5" aria-hidden="true">/</span>
			<a href="/packages/@std/{modName}" class="hover:text-aura-text">@std/{modName}</a>
			<span class="mx-1.5" aria-hidden="true">/</span>
			<span class="text-aura-text">{meta?.tagline ?? 'docs'}</span>
		</nav>
	</div>
	<div class="grid gap-8 pt-6 lg:grid-cols-[240px_minmax(0,1fr)_200px]">
		<nav class="hidden lg:block" aria-label="Package docs">
			<div class="sticky top-24 max-h-[calc(100vh-7rem)] overflow-auto text-sm">
				{@render navtree()}
			</div>
		</nav>

		<div class="min-w-0" data-docs-content>
			{#if meta}
				<p class="flex items-center gap-1.5 font-mono text-sm tracking-wide text-aura-orange"><Diamond size={12} />@std/{modName}</p>
			{/if}
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
									? 'text-aura-orange'
									: 'text-aura-muted hover:text-aura-orange'}"
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
			aria-label="Package docs index"
		>
			<div class="mb-3 flex items-center justify-between">
				<span class="flex items-center gap-1 font-mono text-xs text-aura-orange"><Diamond size={11} />@std/{modName}</span>
				<button
					onclick={() => (drawer = false)}
					class="rounded border border-aura-border px-2 py-0.5 font-mono text-xs text-aura-muted"
					aria-label="Close package index"
				>
					<X size={14} />
				</button>
			</div>
			{@render navtree()}
		</nav>
	</div>
{/if}
