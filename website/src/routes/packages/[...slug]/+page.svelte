<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { browser } from '$app/environment';
	import { GitBranch, Diamond, Star, Download } from 'lucide-svelte';
	import ModuleDocs from '$lib/components/ModuleDocs.svelte';
	import { renderMarkdown } from '$lib/docs/markdown';
	import { buildSymbolIndex, linkifyProseHtml } from '$lib/docs/symbollinks';
	import type { DocModule } from '$lib/docs/api';

	let { data } = $props();
	let pkg = $derived(data.pkg);
	let tab = $derived.by(() => {
		if (!browser) return 'overview';
		const t = page.url.searchParams.get('tab');
		return t === 'docs' || t === 'versions' ? t : 'overview';
	});
	let activeVersion = $derived(data.active);
	let copied = $state(false);

	let readme = $derived(renderMarkdown(activeVersion.readme));
	let docModules = $derived.by(() => {
		try {
			const parsed = JSON.parse(activeVersion.docJson) as { modules: DocModule[] };
			return parsed.modules ?? [];
		} catch {
			return [];
		}
	});

	const BUILTIN_TYPES: Record<string, 'class' | 'enum'> = {
		Int: 'class',
		Float: 'class',
		FastFloat: 'class',
		Bool: 'class',
		Void: 'class',
		String: 'class',
		Any: 'class',
		Array: 'class',
		Map: 'class',
		Set: 'class',
		GenRef: 'class',
		Vec4f: 'class',
		Vec4i: 'class',
		Vec2: 'class',
		Result: 'enum',
		Option: 'enum'
	};

	let typeLinks = $derived.by(() => {		const map = new Map<string, string>();
		for (const m of docModules) {
			for (const c of m.classes ?? []) {
				if (!map.has(c.name)) map.set(c.name, `#pkg-class-${c.name}`);
			}
			for (const e of m.enums ?? []) {
				if (!map.has(e.name)) map.set(e.name, `#pkg-enum-${e.name}`);
			}
		}
		const isPrelude = pkg.name === '@std/prelude';
		for (const [t, kind] of Object.entries(BUILTIN_TYPES)) {
			if (!map.has(t)) {
				map.set(t, isPrelude ? `#pkg-${kind}-${t}` : `/packages/@std/prelude#pkg-${kind}-${t}`);
			}
		}
		return map;
	});

	let pkgIndex = $derived(buildSymbolIndex(docModules, (m, a) => `#pkg-${a}`));

	function pkgHref(_moduleName: string, anchor: string): string {
		return `#pkg-${anchor}`;
	}

	function linkedModuleDocs(description: string): string {
		try {
			return linkifyProseHtml(renderMarkdown(description).html, pkgIndex);
		} catch {
			return description;
		}
	}

	function fmtDownloads(n: number): string {
		return n >= 1000 ? (n / 1000).toFixed(1) + 'k' : String(n);
	}

	function fmtDate(ts: number): string {
		return new Date(ts * 1000).toISOString().slice(0, 10);
	}

	function nav(next: { tab?: string; v?: string }) {
		const t = next.tab ?? tab;
		const v = next.v ?? activeVersion.version;
		const base = v === pkg.latest ? `/packages/${pkg.name}` : `/packages/${pkg.name}/${v}`;
		goto(`${base}?tab=${t}`, { keepFocus: true });
	}

	async function copyInstall() {
		try {
			await navigator.clipboard.writeText(`rnx add ${pkg.name}`);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			copied = false;
		}
	}

	function onCopy(e: MouseEvent) {
		const btn = (e.target as HTMLElement).closest('[data-copy]') as HTMLElement | null;
		if (!btn) return;
		const code = btn.getAttribute('data-code') ?? '';
		navigator.clipboard?.writeText(code).then(
			() => {
				btn.textContent = 'copied';
				setTimeout(() => (btn.textContent = 'copy'), 1500);
			},
			() => {}
		);
	}
</script>

<svelte:head>
	<title>{pkg.name} v{activeVersion.version} — Packages</title>
	<meta name="description" content="{pkg.description}" />
	<link rel="canonical" href="https://rasmalai.rovelstars.com/packages/{pkg.name}" />
</svelte:head>

<main class="mx-auto max-w-7xl px-4 pb-16">
	<div class="panel mt-8 p-5">
		<div class="flex flex-wrap items-baseline gap-x-3 gap-y-2">
			<span class="flex items-center gap-2 font-mono text-lg font-bold text-aura-purple"><Diamond size={15} />{pkg.name}</span>
			{#if pkg.versions.length > 1}
				<select
					value={activeVersion.version}
					onchange={(e) => nav({ v: (e.target as HTMLSelectElement).value })}
					class="rounded border border-aura-border bg-aura-surfaceElevated px-2 py-0.5 font-mono text-xs"
					aria-label="Select version"
				>
					{#each pkg.versions as v}
						<option value={v.version}>v{v.version}</option>
					{/each}
				</select>
			{:else}
				<span class="rounded bg-aura-surfaceElevated px-1.5 py-0.5 font-mono text-[11px] text-aura-pink"
					>v{activeVersion.version}</span
				>
			{/if}
			<span class="rounded border border-aura-border px-1.5 py-0.5 font-mono text-[11px] text-aura-muted"
				>{pkg.license}</span
			>
			<span class="tabular ml-auto font-mono text-xs text-aura-muted"
				><Download size={11} class="inline" /> {fmtDownloads(pkg.downloads)} - <Star size={11} class="inline" /> {pkg.stars}</span
			>
		</div>
		<p class="mt-2 text-sm text-aura-muted">{pkg.description}</p>
		<p class="mt-2 font-mono text-xs text-aura-muted">
			by <span class="text-aura-text">@{pkg.author}</span>
			{#if pkg.repository}
				<span aria-hidden="true"> - </span>
				<a href={pkg.repository} rel="external" class="text-aura-cyan hover:underline">repository</a>
			{/if}
		</p>
		<div class="mt-3 flex items-center gap-2">
			<code class="tabular flex-1 overflow-x-auto rounded bg-aura-bg px-3 py-2 font-mono text-[13px] text-aura-cyan"
				>rnx add {pkg.name}</code
			>
			<button
				onclick={copyInstall}
				class="press shrink-0 rounded border border-aura-border px-3 py-2 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
			>
				{copied ? 'copied' : 'copy'}
			</button>
		</div>
	</div>

	<div class="mt-4 flex gap-1 border-b border-aura-border" role="tablist" aria-label="Package views">
		{#each ['overview', 'docs', 'versions'] as t}
			<button
				role="tab"
				aria-selected={tab === t}
				onclick={() => nav({ tab: t })}
				class="press -mb-px border-b-2 px-4 py-2 font-mono text-sm capitalize transition-colors duration-150 {tab === t
					? 'border-aura-purple text-aura-text'
					: 'border-transparent text-aura-muted hover:text-aura-text'}"
			>
				{t}
			</button>
		{/each}
	</div>

	<div class="grid gap-8 lg:grid-cols-[minmax(0,1fr)_240px]">
		<div class="min-w-0 pt-4">
			{#if tab === 'overview'}
				<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events: delegated copy-button handler -->
				<article class="doc-prose" onclick={onCopy}>
					{@html readme.html}
				</article>
			{:else if tab === 'docs'}
				<article class="doc-prose">
					{#each docModules as m}
						<p class="flex items-center gap-1.5 font-mono text-sm tracking-wide text-aura-purple"><Diamond size={12} />package api</p>
						<h1>{pkg.name} - {m.name}</h1>
						{#if m.docs.description}<div class="doc-prose">{@html linkedModuleDocs(m.docs.description)}</div>{/if}
						<ModuleDocs mod={m} idPrefix="pkg-" typeLinks={typeLinks} allMods={docModules} symbolHref={pkgHref} />
					{/each}
					{#if docModules.length === 0}
						<p class="text-sm text-aura-muted">No API reference for this version.</p>
					{/if}
				</article>
			{:else}
				<div class="panel overflow-x-auto">
					<table class="w-full min-w-[560px] border-collapse font-mono text-[13px]">
						<thead>
							<tr class="border-b border-aura-border text-left text-aura-muted">
								<th class="px-4 py-2.5 font-medium">version</th>
								<th class="px-4 py-2.5 font-medium">released</th>
								<th class="px-4 py-2.5 font-medium">sha-256</th>
								<th class="px-4 py-2.5 text-right font-medium">download</th>
							</tr>
						</thead>
						<tbody>
							{#each pkg.versions as v}
								<tr class="border-b border-aura-border last:border-0">
									<td class="px-4 py-2.5 text-aura-purple">v{v.version}</td>
									<td class="tabular px-4 py-2.5 text-aura-muted">{fmtDate(v.createdAt)}</td>
									<td class="tabular px-4 py-2.5 text-aura-muted">{v.checksum.slice(0, 12)}...</td>
									<td class="px-4 py-2.5 text-right">
										<button
											onclick={() => nav({ v: v.version })}
											class="text-aura-cyan hover:underline">view</button
										>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</div>

		<aside class="space-y-4 pt-4 text-sm">
			<div class="panel p-4">
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">repository</p>
				<a
					href={pkg.repository}
					rel="external"
					class="mt-1 flex items-center gap-1.5 text-aura-cyan hover:underline"
				>
					<GitBranch size={14} strokeWidth={1.5} />
					<span class="break-all font-mono text-[13px]">{pkg.repository}</span>
				</a>
			</div>
			<div class="panel p-4">
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">license</p>
				<p class="mt-1">{pkg.license}</p>
			</div>
			<div class="panel p-4">
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">dependencies</p>
				{#if pkg.dependencies.length > 0}
					<ul class="mt-1 space-y-1">
						{#each pkg.dependencies as d}
							<li><a href="/packages/{d}" class="font-mono text-[13px] text-aura-cyan hover:underline">{d}</a></li>
						{/each}
					</ul>
				{:else}
					<p class="mt-1 text-aura-muted">None.</p>
				{/if}
			</div>
			<div class="panel p-4">
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">maintainers</p>
				<p class="mt-1">@{pkg.author}</p>
			</div>
		</aside>
	</div>
</main>
