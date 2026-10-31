<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { STD_MODULES } from '$lib/docs/stdlib';
	import { humanDate } from '$lib/packages-meta';
	import { Search } from 'lucide-svelte';
	import '$lib/styles/registry.css';

	let { data } = $props();

	type FeedPackage = {
		name: string;
		description: string;
		author: string;
		license: string;
		keywords: string[];
		latest: string;
		updatedAt: number;
		versionCount: number;
		dependents: number;
	};

	let query = $state('');
	let licenseFilter = $state('');
	let updatedSinceDays = $state(0);
	let sort = $state<'updated' | 'alpha'>('updated');
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
			const res = await fetch('/api/v0/packages');
			if (!res.ok) return;
			const body = (await res.json()) as { packages?: Array<Record<string, unknown>> };
			if (!Array.isArray(body.packages)) return;
			const map: Record<string, Partial<FeedPackage>> = {};
			for (const p of body.packages) {
				if (typeof p['name'] !== 'string') continue;
				const entry: Partial<FeedPackage> = {};
				if (Array.isArray(p['keywords'])) entry.keywords = (p['keywords'] as unknown[]).filter((k): k is string => typeof k === 'string');
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
				latest: extra.latest ?? p.latest,
				updatedAt: extra.updatedAt ?? p.updatedAt,
				versionCount: extra.versionCount ?? p.versionCount,
				dependents: extra.dependents ?? p.dependents ?? 0
			};
		});
	});

	let searching = $derived(
		query.trim() !== '' || licenseFilter !== '' || updatedSinceDays > 0
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
			if (cutoff > 0 && p.updatedAt < cutoff) return false;
			if (!q) return true;
			return (
				p.name.toLowerCase().includes(q) ||
				p.description.toLowerCase().includes(q) ||
				p.keywords.some((k) => k.toLowerCase().includes(q))
			);
		});
		list = [...list].sort((a, b) => {
			if (sort === 'updated') return b.updatedAt - a.updatedAt;
			return a.name.localeCompare(b.name);
		});
		return list;
	});

	let headTitle = $derived(searching ? 'Results' : 'All packages');
	let headNote = $derived(
		results.length === 0
			? 'nothing here'
			: searching
				? `${results.length} ${results.length === 1 ? 'package' : 'packages'}${query.trim() ? ` matching "${query.trim()}"` : ''}`
				: `${results.length} ${results.length === 1 ? 'package' : 'packages'} in the registry`
	);

	function isoDate(ts: number): string {
		if (!ts) return '';
		return new Date(ts * 1000).toISOString().slice(0, 10);
	}

	function avatarInitial(name: string): string {
		const handle = name.includes('/') ? (name.split('/').pop() ?? name) : name;
		const clean = handle.replace(/^@/, '');
		return (clean[0] ?? '?').toUpperCase();
	}
</script>

<svelte:head>
	<title>Packages | Rasmalai</title>
	<meta
		name="description"
		content="Search, filter, and browse packages published for the Rasmalai toolchain."
	/>
</svelte:head>

<main class="reg-page">
	<header class="reg-mast shell">
		<div class="reg-mast-main">
			<h1 class="reg-title">Packages</h1>
			<p class="reg-lede">
				Search, filter, and browse libraries published for the Rasmalai toolchain.
			</p>
		</div>
		<button onclick={() => (publishOpen = true)} class="reg-publish">
			Publish (preview)
		</button>
	</header>

	<div class="shell">
		<div class="reg-searchbar" role="search">
			<Search size={18} class="reg-search-icon" />
			<input
				id="package-search"
				bind:value={query}
				oninput={onSearchInput}
				placeholder="Search packages by name, word, or keyword"
				aria-label="Search packages"
				autocomplete="off"
			/>
			{#if query.trim()}
				<button
					class="reg-clear"
					onclick={() => {
						query = '';
						syncUrl();
					}}
				>
					Clear
				</button>
			{/if}
		</div>
	</div>

	<div class="shell reg-desk">
		<aside class="reg-rail">
			<section class="reg-block" aria-label="Filters">
				<p class="reg-rail-title">Filters</p>
				<div class="reg-fields">
					<label class="reg-field">
						<span>License</span>
						<select bind:value={licenseFilter}>
							<option value="">Any license</option>
							{#each licenses as l}
								<option value={l}>{l}</option>
							{/each}
						</select>
					</label>
					<label class="reg-field">
						<span>Updated</span>
						<select bind:value={updatedSinceDays}>
							<option value={0}>Anytime</option>
							<option value={7}>Past week</option>
							<option value={30}>Past month</option>
							<option value={90}>Past 3 months</option>
							<option value={365}>Past year</option>
						</select>
					</label>
					<label class="reg-field">
						<span>Order</span>
						<select bind:value={sort}>
							<option value="updated">Recently updated</option>
							<option value="alpha">Alphabetical</option>
						</select>
					</label>
				</div>
			</section>

			<section class="reg-block" aria-label="Keywords">
				<p class="reg-rail-title">Keywords</p>
				{#if keywordCounts.length > 0}
					<ul class="reg-keywords">
						{#each keywordCounts.slice(0, 14) as [k, n]}
							<li>
								<button onclick={() => searchFor(k)}>
									<span>{k}</span>
									<span class="reg-kw-count">{n}</span>
								</button>
							</li>
						{/each}
					</ul>
				{:else}
					<ul class="reg-keywords">
						{#each FALLBACK_KEYWORDS as k}
							<li>
								<button onclick={() => searchFor(k)}>
									<span>{k}</span>
									<span class="reg-kw-count">0</span>
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</section>

			<dl class="reg-stats">
				<div>
					<dt>Published</dt>
					<dd>{feed.length}</dd>
				</div>
				<div>
					<dt>Keywords</dt>
					<dd>{keywordCounts.length}</dd>
				</div>
				<div>
					<dt>Standard modules</dt>
					<dd>{STD_MODULES.length}</dd>
				</div>
			</dl>
		</aside>

		<div class="reg-main">
			<section class="reg-group" aria-labelledby="catalog-heading">
				<header class="reg-group-head">
					<h2 id="catalog-heading">{headTitle}</h2>
					<p aria-live="polite">{headNote}</p>
				</header>
				{#if results.length > 0}
					<ol class="reg-list">
						{#each results as p, i}
						<li class="reg-row">
							<span class="reg-index">{String(i + 1).padStart(2, '0')}</span>
							<div class="reg-row-main">
								<div class="reg-row-title">
									<a href="/packages/{p.name}" class="reg-name">{p.name}</a>
									<span class="reg-version">v{p.latest}</span>
									<span class="reg-license">{p.license}</span>
								</div>
								<p class="reg-desc">{p.description}</p>
								{#if p.keywords.length > 0}
									<div class="reg-tags" aria-label="Keywords">
										{#each p.keywords.slice(0, 6) as k}
											<button onclick={() => searchFor(k)}>{k}</button>
										{/each}
									</div>
								{/if}
							</div>
							<div class="reg-row-meta">
								<span class="reg-author">
									<span class="reg-avatar" aria-hidden="true">{avatarInitial(p.author)}</span>
									<span>{p.author}</span>
								</span>
								<span class="reg-date" title={isoDate(p.updatedAt)}>{humanDate(p.updatedAt)}</span>
							</div>
						</li>
					{/each}
				</ol>
				{:else}
					<p class="reg-empty">
						{#if feed.length === 0}
							The registry is empty. Publish the first package with <code>rnx publish</code>.
						{:else}
							No packages match. Try a different search or clear the filters.
						{/if}
					</p>
				{/if}
				{#if !searching}
					<p class="reg-std-note">
						The standard library ships through this registry. The full reference lives under
						<a href="/docs/@std/prelude/overview">docs</a>.
					</p>
				{/if}
			</section>
		</div>
	</div>
</main>
{#if publishOpen}
	<!-- svelte-ignore a11y_click_events_have_key_events: backdrop has an Escape key handler below -->
	<div
		class="reg-modal-backdrop"
		onclick={() => (publishOpen = false)}
		onkeydown={(e) => {
			if (e.key === 'Escape') publishOpen = false;
		}}
		role="presentation"
	>
		<div
			class="reg-modal modal-pop"
			role="dialog"
			tabindex="-1"
			aria-modal="true"
			aria-label="Publish preview"
			onclick={(e) => e.stopPropagation()}
		>
			<h2>Ship your package</h2>
			<p class="reg-modal-note">
				Publishing is limited to the Rovel Stars org while account logins are being built.
				Scoped names use the <code>@org/name</code> form.
			</p>
			<pre class="reg-modal-code"><code>rnx pack
rnx publish --token &lt;publisher-token&gt;</code></pre>
			<button onclick={() => (publishOpen = false)} class="reg-modal-close">
				Close
			</button>
		</div>
	</div>
{/if}
