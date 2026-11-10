<script lang="ts">
	import { ArrowLeft } from 'lucide-svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import DocsShell from '$lib/components/DocsShell.svelte';
	import { STDLIB_ICONS } from '$lib/docs/nav';
	import { buildPackagePageNav } from '$lib/docs/package-pages';
	import { ENGINE_VERSION, stdApiModule, quickstartFor } from '$lib/docs/stdlib';
	import { encodeSnippet } from '$lib/playground/share';

	let { children } = $props();

	let modName = $derived((page.data.module as string | undefined) ?? page.params.name ?? '');
	let scope = $derived((page.data.scope as string | undefined) ?? page.params.scope ?? '');
	let full = $derived(page.data.full ?? `${scope}/${modName}`);
	let Icon = $derived(modName ? STDLIB_ICONS[modName] : null);

	let symbols = $derived.by(() => {
		const mod = (page.data.mod ?? (scope === '@std' ? stdApiModule(modName) : null)) as {
			classes: Array<{ name: string }>;
			enums: Array<{ name: string }>;
			functions: Array<{ name: string }>;
		} | null;
		if (!mod) return [];
		return [
			...mod.classes.map((c) => ({ kind: 'Class', name: c.name, href: 'api' as const, anchor: `class-${c.name}` })),
			...mod.enums.map((e) => ({ kind: 'Enum', name: e.name, href: 'api' as const, anchor: `enum-${e.name}` })),
			...mod.functions.map((f) => ({ kind: 'Fn', name: f.name, href: 'api' as const, anchor: `fn-${f.name}` }))
		];
	});

	const SECTIONS = [
		{ slug: 'overview', title: 'Overview' },
		{ slug: 'getting-started', title: 'Getting started' }
	];

	let currentSection = $derived((page.data.page as string | undefined) ?? page.params.page ?? 'overview');
	type GuideLink = { slug: string; title: string; section?: string | null; order?: string | null };
	const basePages = new Set(['overview', 'getting-started', 'api']);
	let guides = $derived(
		((page.data.guides ?? []) as GuideLink[]).filter((g) => !basePages.has(g.slug))
	);
	let guideNav = $derived(buildPackagePageNav(guides));
	let tryCode = $derived.by(() => {
		if (scope !== '@std') return null;
		try {
			return encodeSnippet(quickstartFor(modName));
		} catch {
			return null;
		}
	});
	let version = $derived((page.data.version as string | undefined) ?? ENGINE_VERSION);
	let versions = $derived((page.data.versions as string[] | undefined) ?? [version]);
	let latest = $derived((page.data.latest as string | undefined) ?? version);
	function onVersionChange(event: Event) {
		const next = (event.currentTarget as HTMLSelectElement).value;
		const url = new URL(page.url);
		if (next === latest) url.searchParams.delete('v');
		else url.searchParams.set('v', next);
		goto(url, { keepFocus: true });
	}
</script>

{#snippet nav()}
	<p class="docs-group">Package</p>
	<ul class="docs-list">
		<li>
			<a href="/packages/{full}" class="docs-icon">
				<ArrowLeft size={13} />
				Package page
			</a>
		</li>
	</ul>

	<div class="mt-4 border-y border-aura-border py-3">
		<p class="flex items-center gap-2 font-mono text-[13px] text-aura-text">
			{#if Icon}<Icon size={14} strokeWidth={1.75} class="shrink-0 text-aura-orange" />{/if}
			<span class="min-w-0 truncate">{full}</span>
		</p>
		<div class="mt-2 flex flex-wrap items-center gap-2 font-mono text-[11px]">
			{#if versions.length > 1}
				<select
					value={version}
					onchange={onVersionChange}
					aria-label="Select version"
					class="docs-version"
				>
					{#each versions as v}
						<option value={v}>v{v}</option>
					{/each}
				</select>
			{:else}
				<span class="text-aura-faint" title="Documented version">v{version}</span>
			{/if}
			{#if tryCode}
				<a
					href="/playground?code={tryCode}"
					target="_blank"
					rel="noopener"
					class="text-aura-green hover:underline"
				>
					try it
				</a>
			{/if}
		</div>
	</div>

	<p class="docs-group">Pages</p>
	<ul class="docs-list">
		{#each SECTIONS as s}
			{@const active = currentSection === s.slug}
			<li>
				<a href="/docs/{full}/{s.slug}" aria-current={active ? 'page' : undefined}>{s.title}</a>
			</li>
		{/each}
		{#each guideNav.loose as guide}
			{@const active = currentSection === guide.slug}
			<li>
				<a href="/docs/{full}/{guide.slug}" aria-current={active ? 'page' : undefined}>
					{guide.title}
				</a>
			</li>
		{/each}
	</ul>

	{#each guideNav.sections as { section, pages }}
		<p class="docs-group">{section}</p>
		<ul class="docs-list">
			{#each pages as guide}
				{@const active = currentSection === guide.slug}
				<li>
					<a href="/docs/{full}/{guide.slug}" aria-current={active ? 'page' : undefined}>
						{guide.title}
					</a>
				</li>
			{/each}
		</ul>
	{/each}

	<p class="docs-group">API Reference</p>
	<ul class="docs-list">
		<li>
			<a
				href="/docs/{full}/api"
				aria-current={currentSection === 'api' ? 'page' : undefined}
			>
				All symbols
			</a>
		</li>
		{#each symbols as s}
			<li>
				<a href="/docs/{full}/{s.href}#{s.anchor}" class="font-mono text-[12px]">
					<span class="text-aura-pink">{s.kind[0]}</span>
					<span class="ml-1.5">{s.name}</span>
				</a>
			</li>
		{/each}
	</ul>
{/snippet}

{#snippet header()}
	<nav class="docs-crumb" aria-label="Breadcrumb">
		<a href="/packages">Packages</a>
		<span class="sep" aria-hidden="true">/</span>
		<a href="/packages/{full}">{full}</a>
	</nav>
{/snippet}

<DocsShell label="Package docs" drawerLabel="Package" toc={page.data.toc ?? []} {nav} {header}>
	{@render children()}
</DocsShell>
