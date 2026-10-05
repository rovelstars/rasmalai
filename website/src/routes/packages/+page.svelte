<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { STD_MODULES } from '$lib/docs/stdlib';
	import { shortDownloads, humanDate } from '$lib/packages-meta';

	let { data } = $props();

	type FeedPackage = {
		name: string;
		description: string;
		author: string;
		license: string;
		keywords: string[];
		downloads: number;
		latest: string;
		updatedAt: number;
		versionCount: number;
		dependents: number;
	};

	let query = $state('');
	let licenseFilter = $state('');
	let minDownloads = $state(0);
	let updatedSinceDays = $state(0);
	let sort = $state<'downloads' | 'updated' | 'alpha'>('downloads');
	let publishOpen = $state(false);
	let extraByName = $state<Record<string, Partial<FeedPackage>>>({});
	let urlReady = $state(false);

	onMount(() => {
		const initial = page.url.searchParams.get('search') ?? '';
		if (initial) query = initial;
		urlReady = true;
		refreshExtended();
	});

	async function refreshExtended() {
		try {
			const res = await fetch('/api/packages');
			if (!res.ok) return;
			const body = (await res.json()) as { packages?: Array<Record<string, unknown>> };
			if (!Array.isArray(body.packages)) return;
			const map: Record<string, Partial<FeedPackage>> = {};
			for (const p of body.packages) {
				if (typeof p['name'] !== 'string') continue;
				const entry: Partial<FeedPackage> = {};
				if (Array.isArray(p['keywords'])) entry.keywords = (p['keywords'] as unknown[]).filter((k): k is string => typeof k === 'string');
				if (typeof p['downloads'] === 'number') entry.downloads = p['downloads'];
				if (typeof p['latest'] === 'string') entry.latest = p['latest'];
				if (typeof p['updatedAt'] === 'number') entry.updatedAt = p['updatedAt'];
				if (typeof p['versionCount'] === 'number') entry.versionCount = p['versionCount'];
				if (typeof p['dependents'] === 'number') entry.dependents = p['dependents'];
				if (typeof p['author'] === 'string') entry.author = p['author'];
				if (typeof p['license'] === 'string') entry.license = p['license'];
				if (typeof p['description'] === 'string') entry.description = p['description'];
				map[p['name']] = entry;
			}
			extraByName = map;
		} catch {
			extraByName = {};
		}
	}

	function syncUrl() {
		if (!urlReady || typeof window === 'undefined') return;
		const url = new URL(window.location.href);
		if (query.trim()) url.searchParams.set('search', query.trim());
		else url.searchParams.delete('search');
		window.history.replaceState({}, '', url);
	}

	function onSearchInput() {
		syncUrl();
	}

	function searchFor(term: string) {
		query = term;
		licenseFilter = '';
		minDownloads = 0;
		updatedSinceDays = 0;
		syncUrl();
		const el = document.getElementById('package-search');
		el?.focus();
		window.scrollTo({ top: 0, behavior: 'smooth' });
	}

	let feed = $derived.by((): FeedPackage[] => {
		return data.packages.map((p) => {
			const extra = extraByName[p.name] ?? {};
			const tags = Array.isArray(p.tags) ? p.tags : [];
			const kw = Array.isArray(p.keywords) && p.keywords.length > 0 ? p.keywords : tags;
			return {
				name: p.name,
				description: extra.description ?? p.description,
				author: extra.author ?? p.author,
				license: extra.license ?? p.license,
				keywords: extra.keywords ?? kw,
				downloads: extra.downloads ?? p.downloads,
				latest: extra.latest ?? p.latest,
				updatedAt: extra.updatedAt ?? p.updatedAt,
				versionCount: extra.versionCount ?? p.versionCount,
				dependents: extra.dependents ?? p.dependents ?? 0
			};
		});
	});

	let searching = $derived(
		query.trim() !== '' || licenseFilter !== '' || minDownloads > 0 || updatedSinceDays > 0
	);

	let keywordCounts = $derived.by(() => {
		const counts = new Map<string, number>();
		for (const p of feed) {
			for (const k of new Set(p.keywords)) {
				counts.set(k, (counts.get(k) ?? 0) + 1);
			}
		}
		return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
	});

	const FALLBACK_KEYWORDS = ['http', 'json', 'cli', 'crypto', 'math', 'async'];

	let licenses = $derived.by(() => {
		const set = new Set<string>();
		for (const p of feed) if (p.license) set.add(p.license);
		return [...set].sort();
	});

	let results = $derived.by(() => {
		const q = query.trim().toLowerCase();
		const cutoff = updatedSinceDays > 0 ? Date.now() / 1000 - updatedSinceDays * 86400 : 0;
		let list = feed.filter((p) => {
			if (licenseFilter && p.license !== licenseFilter) return false;
			if (p.downloads < minDownloads) return false;
			if (cutoff > 0 && p.updatedAt < cutoff) return false;
			if (!q) return true;
			return (
				p.name.toLowerCase().includes(q) ||
				p.description.toLowerCase().includes(q) ||
				p.keywords.some((k) => k.toLowerCase().includes(q))
			);
		});
		list = [...list].sort((a, b) => {
			if (sort === 'downloads') return b.downloads - a.downloads;
			if (sort === 'updated') return b.updatedAt - a.updatedAt;
			return a.name.localeCompare(b.name);
		});
		return list;
	});

	let popular = $derived([...feed].sort((a, b) => b.downloads - a.downloads).slice(0, 8));
	let totalDownloads = $derived(feed.reduce((sum, p) => sum + p.downloads, 0));

	function isoDate(ts: number): string {
		if (!ts) return '';
		return new Date(ts * 1000).toISOString().slice(0, 10);
	}

	function avatarHue(name: string): number {
		let h = 0;
		for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
		return h % 360;
	}

	function avatarInitial(name: string): string {
		const handle = name.includes('/') ? (name.split('/').pop() ?? name) : name;
		const clean = handle.replace(/^@/, '');
		return (clean[0] ?? '?').toUpperCase();
	}
</script>

<svelte:head>
	<title>Packages — Rasmalai</title>
</svelte:head>

<main class="mx-auto max-w-7xl px-4 pb-16">
	<div class="flex flex-wrap items-end justify-between gap-4 pt-6">
		<div>
			<h1 class="text-3xl font-bold tracking-tight">Packages</h1>
			<p class="mt-2 max-w-[65ch] text-aura-muted">
				Libraries published for the Rasmalai toolchain. Search by name, description, or
				keyword — or browse what the community downloads most.
			</p>
		</div>
		<button
			onclick={() => (publishOpen = true)}
			class="press rounded-md bg-aura-purple px-4 py-2 text-sm font-semibold text-[#15141b]"
		>
			+ Publish (Preview)
		</button>
	</div>

	<div class="mt-6" role="search">
		<input
			id="package-search"
			bind:value={query}
			oninput={onSearchInput}
			placeholder="Search packages — try a name, a word, or a keyword…"
			class="w-full rounded-md border border-aura-border bg-aura-surface px-3 py-2 text-sm outline-none placeholder:text-aura-muted focus:border-aura-borderHover"
			aria-label="Search packages"
			autocomplete="off"
		/>
	</div>

	{#if searching}
		<div class="mt-4 flex flex-wrap items-center gap-2" aria-label="Result filters">
			<span class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">filter</span>
			<select
				bind:value={licenseFilter}
				class="rounded-md border border-aura-border bg-aura-surface px-2.5 py-1.5 text-sm"
				aria-label="Filter by license"
			>
				<option value="">Any license</option>
				{#each licenses as l}
					<option value={l}>{l}</option>
				{/each}
			</select>
			<select
				bind:value={minDownloads}
				class="rounded-md border border-aura-border bg-aura-surface px-2.5 py-1.5 text-sm"
				aria-label="Minimum downloads"
			>
				<option value={0}>Any downloads</option>
				<option value={100}>100+ downloads</option>
				<option value={1000}>1k+ downloads</option>
				<option value={10000}>10k+ downloads</option>
			</select>
			<select
				bind:value={updatedSinceDays}
				class="rounded-md border border-aura-border bg-aura-surface px-2.5 py-1.5 text-sm"
				aria-label="Updated since"
			>
				<option value={0}>Updated anytime</option>
				<option value={7}>Past week</option>
				<option value={30}>Past month</option>
				<option value={90}>Past 3 months</option>
				<option value={365}>Past year</option>
			</select>
			<select
				bind:value={sort}
				class="rounded-md border border-aura-border bg-aura-surface px-2.5 py-1.5 text-sm"
				aria-label="Sort packages"
			>
				<option value="downloads">Most downloads</option>
				<option value="updated">Recently updated</option>
				<option value="alpha">Alphabetical</option>
			</select>
			{#if keywordCounts.length > 0}
				<span class="hidden h-5 w-px bg-aura-border sm:inline-block" aria-hidden="true"></span>
				<div class="flex flex-wrap gap-1.5" role="group" aria-label="Filter by keyword">
					{#each keywordCounts.slice(0, 10) as [k]}
						<button
							onclick={() => searchFor(k)}
							class="press rounded-full border border-aura-border px-2.5 py-0.5 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
						>
							{k}
						</button>
					{/each}
				</div>
			{/if}
		</div>

		<p class="tabular mt-4 font-mono text-xs text-aura-muted" aria-live="polite">
			{results.length} {results.length === 1 ? 'result' : 'results'}{query.trim()
				? ` for “${query.trim()}”`
				: ''}
		</p>
		<ul class="mt-3 space-y-2.5">
			{#each results as p}
				<li class="panel p-4 hover:border-aura-borderHover">
					<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
						<a
							href="/packages/{p.name}"
							class="font-mono font-bold text-aura-text hover:text-aura-purple">{p.name}</a
						>
						<span class="rounded bg-aura-surfaceElevated px-1.5 py-0.5 font-mono text-[11px] text-aura-muted"
							>v{p.latest}</span
						>
						<span class="tabular ml-auto font-mono text-xs text-aura-muted">
							{shortDownloads(p.downloads)} downloads · {p.license}
						</span>
					</div>
					<p class="mt-1 text-sm text-aura-muted">{p.description}</p>
					<div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
						{#if p.keywords.length > 0}
							<span class="flex flex-wrap gap-1.5" aria-label="Keywords">
								{#each p.keywords.slice(0, 6) as k}
									<button
										onclick={() => searchFor(k)}
										class="rounded-full border border-aura-border px-2 py-px font-mono text-[11px] text-aura-purple hover:border-aura-borderHover"
									>
										{k}
									</button>
								{/each}
							</span>
						{/if}
						<span class="ml-auto flex items-center gap-1.5 text-xs text-aura-muted">
							<span
								class="flex h-5 w-5 items-center justify-center rounded-full font-mono text-[11px] font-bold text-white"
								style="background-color: hsl({avatarHue(p.author)} 45% 38%)"
								aria-hidden="true">{avatarInitial(p.author)}</span
							>
							<span>{p.author}</span>
							<span aria-hidden="true">·</span>
							<span class="tabular" title={isoDate(p.updatedAt)}>{humanDate(p.updatedAt)}</span>
						</span>
					</div>
				</li>
			{/each}
			{#if results.length === 0}
				<li class="panel p-6 text-center text-sm text-aura-muted">
					{#if feed.length === 0}
						The registry is empty. Publish the first package with <code
							class="font-mono text-aura-cyan">rnx publish</code
						>.
					{:else}
						No packages match. Try a different search or clear the filters.
					{/if}
				</li>
			{/if}
		</ul>
	{:else}
		<section class="mt-8" aria-labelledby="popular-heading">
			<div class="flex items-baseline justify-between gap-4">
				<h2 id="popular-heading" class="text-lg font-bold tracking-tight">Popular libraries</h2>
				<p class="font-mono text-[11px] text-aura-muted">ranked by downloads</p>
			</div>
			{#if popular.length > 0}
				<ol class="panel mt-3 divide-y divide-aura-border">
					{#each popular as p, i}
						<li class="flex items-baseline gap-3 px-4 py-2.5">
							<span class="tabular w-6 shrink-0 text-right font-mono text-xs text-aura-muted">{i + 1}</span>
							<a
								href="/packages/{p.name}"
								class="min-w-0 shrink-0 font-mono text-sm font-bold text-aura-text hover:text-aura-purple"
								>{p.name}</a
							>
							<span class="hidden min-w-0 flex-1 truncate text-sm text-aura-muted md:inline">{p.description}</span>
							<span class="tabular ml-auto shrink-0 font-mono text-xs text-aura-muted"
								>{shortDownloads(p.downloads)} downloads</span
							>
						</li>
					{/each}
				</ol>
			{:else}
				<p class="panel mt-3 p-6 text-center text-sm text-aura-muted">
					The registry is empty. Publish the first package with <code
						class="font-mono text-aura-cyan">rnx publish</code
					>.
				</p>
			{/if}
		</section>

		<section class="mt-10" aria-labelledby="keywords-heading">
			<h2 id="keywords-heading" class="text-lg font-bold tracking-tight">Discover by keyword</h2>
			{#if keywordCounts.length > 0}
				<ul class="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4">
					{#each keywordCounts.slice(0, 12) as [k, n]}
						<li>
							<button
								onclick={() => searchFor(k)}
								class="press flex w-full items-baseline justify-between gap-2 rounded-md border border-aura-border bg-aura-surface px-3 py-2.5 text-left hover:border-aura-borderHover"
							>
								<span class="font-mono text-sm text-aura-purple">{k}</span>
								<span class="tabular shrink-0 font-mono text-[11px] text-aura-muted"
									>{n} {n === 1 ? 'pkg' : 'pkgs'}</span
								>
							</button>
						</li>
					{/each}
				</ul>
			{:else}
				<ul class="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
					{#each FALLBACK_KEYWORDS as k}
						<li>
							<button
								onclick={() => searchFor(k)}
								class="press w-full rounded-md border border-aura-border bg-aura-surface px-3 py-2.5 text-left font-mono text-sm text-aura-purple hover:border-aura-borderHover"
							>
								{k}
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section class="mt-10" aria-labelledby="numbers-heading">
			<h2 id="numbers-heading" class="text-lg font-bold tracking-tight">By the numbers</h2>
			<dl class="mt-3 grid gap-2 sm:grid-cols-2">
				<div class="panel px-4 py-5 text-center">
					<dt class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">packages</dt>
					<dd class="tabular mt-1 font-mono text-2xl font-bold">{feed.length}</dd>
				</div>
				<div class="panel px-4 py-5 text-center">
					<dt class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">downloads, last 30 days</dt>
					<dd class="tabular mt-1 font-mono text-2xl font-bold">{shortDownloads(totalDownloads)}</dd>
				</div>
			</dl>
		</section>

		<section class="mt-10" aria-labelledby="std-heading">
			<h2 id="std-heading" class="text-lg font-bold tracking-tight">Standard library</h2>
			<p class="mt-1 text-sm text-aura-muted">
				Ships with the compiler — nothing to install. Full reference lives under
				<a href="/docs/@std/prelude/overview" class="text-aura-purple hover:underline">docs</a>.
			</p>
			<ul class="mt-3 flex flex-wrap gap-1.5">
				{#each STD_MODULES as m}
					<li>
						<a
							href="/packages/@std/{m.name}"
							class="inline-block rounded-full border border-aura-border px-2.5 py-0.5 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
							>@std/{m.name}</a
						>
					</li>
				{/each}
			</ul>
		</section>
	{/if}
</main>

{#if publishOpen}
	<!-- svelte-ignore a11y_click_events_have_key_events: backdrop has an Escape key handler below -->
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
		onclick={() => (publishOpen = false)}
		onkeydown={(e) => {
			if (e.key === 'Escape') publishOpen = false;
		}}
		role="presentation"
	>
		<div
			class="panel w-full max-w-md p-6"
			role="dialog"
			tabindex="-1"
			aria-modal="true"
			aria-label="Publish preview"
			onclick={(e) => e.stopPropagation()}
		>
			<p class="font-mono text-xs tracking-wide text-aura-purple">publish</p>
			<h2 class="mt-2 text-xl font-semibold">Ship your package</h2>
			<pre class="mt-3 overflow-x-auto rounded border border-aura-border bg-aura-bg p-3 font-mono text-[13px] leading-relaxed">To publish a package:
1. $ rnx pack
2. $ rnx publish --token &lt;publisher-token&gt;
Publishing is currently limited to the Rovel Stars org while account
logins are being built. Scoped names look like @rovelstars/name.</pre>
			<button
				onclick={() => (publishOpen = false)}
				class="press mt-5 w-full rounded-md border border-aura-border px-4 py-2 text-sm hover:border-aura-borderHover"
			>
				Close
			</button>
		</div>
	</div>
{/if}
