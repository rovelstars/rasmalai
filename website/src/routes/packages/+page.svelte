<script lang="ts">
	import { onMount } from 'svelte';
	import { ArrowRight, GitBranch, Diamond, Star, Download } from 'lucide-svelte';
	import { STDLIB_ICONS } from '$lib/docs/nav';
	import { STD_MODULES, ENGINE_VERSION, ensureApi, hasApi, importSnippet, stdApiModule } from '$lib/docs/stdlib';

	let { data } = $props();

	let query = $state('');
	let sort = $state<'downloads' | 'updated' | 'alpha'>('downloads');
	let activeTag = $state<string | null>(null);
	let publishOpen = $state(false);
	let copiedName = $state<string | null>(null);
	let copiedImport = $state<string | null>(null);
	let apiState = $state<'loading' | 'ready' | 'missing'>('loading');

	onMount(async () => {
		await ensureApi();
		apiState = hasApi() ? 'ready' : 'missing';
	});

	const ALL_TAGS = ['simd', 'math', 'network', 'crypto', 'cli'];

	function fmtDownloads(n: number): string {
		return n >= 1000 ? (n / 1000).toFixed(1) + 'k' : String(n);
	}

	function fmtDate(ts: number): string {
		return new Date(ts * 1000).toISOString().slice(0, 10);
	}

	let visible = $derived.by(() => {
		const q = query.trim().toLowerCase();
		let list = data.packages.filter((p) => {
			if (activeTag && !p.tags.includes(activeTag)) return false;
			if (!q) return true;
			return (
				p.name.toLowerCase().includes(q) ||
				p.description.toLowerCase().includes(q) ||
				p.tags.some((t) => t.toLowerCase().includes(q))
			);
		});
		list = [...list].sort((a, b) => {
			if (sort === 'downloads') return b.downloads - a.downloads;
			if (sort === 'updated') return b.updatedAt - a.updatedAt;
			return a.name.localeCompare(b.name);
		});
		return list;
	});

	let stdVisible = $derived.by(() => {
		const q = query.trim().toLowerCase();
		return STD_MODULES.filter((m) => {
			if (activeTag) return false;
			if (!q) return true;
			return (
				`@std/${m.name}`.includes(q) ||
				m.tagline.toLowerCase().includes(q) ||
				m.primaryExport.toLowerCase().includes(q)
			);
		});
	});

	function symbolCount(name: string): number {
		const mod = stdApiModule(name);
		if (!mod) return 0;
		return mod.classes.length + mod.enums.length + mod.functions.length + mod.constants.length;
	}

	async function copyImport(name: string) {
		try {
			await navigator.clipboard.writeText(importSnippet(name));
			copiedImport = name;
			setTimeout(() => {
				if (copiedImport === name) copiedImport = null;
			}, 1500);
		} catch {
			copiedImport = null;
		}
	}

	async function copyInstall(name: string) {
		try {
			await navigator.clipboard.writeText(`rnx add ${name}`);
			copiedName = name;
			setTimeout(() => {
				if (copiedName === name) copiedName = null;
			}, 1500);
		} catch {
			copiedName = null;
		}
	}
</script>

<svelte:head>
	<title>Packages — Rasmalai</title>
</svelte:head>

<main class="mx-auto max-w-7xl px-4 pb-16">
	<div class="flex flex-wrap items-end justify-between gap-4 pt-8">
		<div>
			<h1 class="text-3xl font-bold tracking-tight">Packages</h1>
			<p class="mt-2 max-w-[65ch] text-aura-muted">
				Community libraries for the Rasmalai toolchain. Preview data shown until the
				registry opens in v1.1.
			</p>
		</div>
		<button
			onclick={() => (publishOpen = true)}
			class="press rounded-md bg-aura-purple px-4 py-2 text-sm font-semibold text-[#15141b]"
		>
			+ Publish (Preview)
		</button>
	</div>

	<div class="mt-6 flex flex-wrap items-center gap-2">
		<input
			bind:value={query}
			placeholder="search name, description, tags..."
			class="min-w-0 flex-1 rounded-md border border-aura-border bg-aura-surface px-3 py-1.5 text-sm outline-none placeholder:text-aura-muted focus:border-aura-borderHover"
			aria-label="Search packages"
		/>
		<select
			bind:value={sort}
			class="rounded-md border border-aura-border bg-aura-surface px-2.5 py-1.5 text-sm"
			aria-label="Sort packages"
		>
			<option value="downloads">Most downloads</option>
			<option value="updated">Recently updated</option>
			<option value="alpha">Alphabetical</option>
		</select>
	</div>
	<div class="mt-3 flex flex-wrap gap-1.5" role="group" aria-label="Filter by tag">
		{#each ALL_TAGS as t}
			<button
				onclick={() => (activeTag = activeTag === t ? null : t)}
				class="press rounded-full border px-2.5 py-0.5 font-mono text-xs {activeTag === t
					? 'border-aura-borderHover bg-aura-surfaceElevated text-aura-purple'
					: 'border-aura-border text-aura-muted hover:text-aura-text'}"
				aria-pressed={activeTag === t}
			>
				{t}
			</button>
		{/each}
	</div>

	<ul class="mt-6 space-y-3">
		{#if stdVisible.length > 0}
			<li>
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">
					standard library - ships with the compiler
				</p>
				<div class="mt-2 grid gap-3 md:grid-cols-2">
					{#each stdVisible as m}
						{@const Icon = STDLIB_ICONS[m.name]}
						<div class="panel p-4 hover:border-aura-borderHover">
							<div class="flex flex-wrap items-center gap-x-2.5 gap-y-1">
								{#if Icon}<Icon size={15} strokeWidth={1.75} class="shrink-0 text-aura-orange" />{/if}
								<a
									href="/packages/@std/{m.name}"
									class="font-mono text-sm font-bold text-aura-text hover:text-aura-purple"
									>@std/{m.name}</a
								>
								<span class="rounded bg-aura-surfaceElevated px-1.5 py-0.5 font-mono text-[11px] text-aura-green"
									>standard library</span
								>
								<span class="tabular ml-auto font-mono text-[11px] text-aura-muted"
									>engine >= {ENGINE_VERSION}</span
								>
							</div>
							<p class="mt-1 text-sm text-aura-muted">{m.tagline}</p>
							<div class="mt-2 flex flex-wrap items-center gap-2">
								<code class="tabular min-w-0 flex-1 overflow-x-auto font-mono text-xs text-aura-cyan"
									>{importSnippet(m.name)}</code
								>
								<button
									onclick={() => copyImport(m.name)}
									class="shrink-0 rounded border border-aura-border px-2 py-0.5 font-mono text-[11px] text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
								>
									{copiedImport === m.name ? 'copied' : 'copy'}
								</button>
							</div>
							<p class="tabular mt-2 font-mono text-[11px] text-aura-muted">
								{#if apiState === 'ready'}
									{symbolCount(m.name)} symbols - <a
										href="/docs/@std/{m.name}/overview"
										class="inline-flex items-center gap-1 text-aura-purple hover:underline"
										>docs<ArrowRight size={12} /></a
									>
								{:else if apiState === 'loading'}
									loading symbols...
								{:else}
									No API data yet - <a
										href="/docs/@std/{m.name}/overview"
										class="inline-flex items-center gap-1 text-aura-purple hover:underline"
										>docs<ArrowRight size={12} /></a
									>
								{/if}
							</p>
						</div>
					{/each}
				</div>
			</li>
		{/if}
		{#if visible.length > 0}
			<li>
				<p class="mt-2 font-mono text-[11px] uppercase tracking-wider text-aura-muted">
					community
				</p>
			</li>
		{/if}
		{#each visible as p}
			<li class="panel p-4 hover:border-aura-borderHover">
				<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
					<a
						href="/packages/{p.name}"
						class="font-mono font-bold text-aura-text hover:text-aura-purple">{p.name}</a
					>
					<span class="rounded bg-aura-surfaceElevated px-1.5 py-0.5 font-mono text-[11px] text-aura-pink"
						>v{p.latest}</span
					>
					<span class="text-sm text-aura-muted">by {p.author}</span>
					<span class="tabular ml-auto font-mono text-xs text-aura-muted">
						<Download size={11} class="inline" /> {fmtDownloads(p.downloads)} - <Star size={11} class="inline" /> {p.stars}
					</span>
				</div>
				<p class="mt-1 text-sm text-aura-muted">{p.description}</p>
				<div class="mt-2 flex flex-wrap items-center gap-2">
					<code class="tabular font-mono text-xs text-aura-cyan">rnx add {p.name}</code>
					<button
						onclick={() => copyInstall(p.name)}
						class="rounded border border-aura-border px-2 py-0.5 font-mono text-[11px] text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
					>
						{copiedName === p.name ? 'copied' : 'copy'}
					</button>
					<span class="ml-auto font-mono text-[11px] text-aura-muted"
						>updated {fmtDate(p.updatedAt)}</span
					>
				</div>
			</li>
		{/each}
		{#if visible.length === 0 && stdVisible.length === 0}
			<li class="panel p-6 text-center text-sm text-aura-muted">
				{#if data.packages.length === 0}
					The registry is empty. Publish the first package with <code
						class="font-mono text-aura-cyan">rnx publish</code
					>.
				{:else}
					No packages match.
				{/if}
			</li>
		{/if}
	</ul>
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
			<p class="flex items-center gap-1.5 font-mono text-xs tracking-wide text-aura-purple"><Diamond size={11} />publish</p>
			<h2 class="mt-2 text-xl font-semibold">Ship your package</h2>
			<pre class="mt-3 overflow-x-auto rounded border border-aura-border bg-aura-bg p-3 font-mono text-[13px] leading-relaxed">To publish a package:
1. $ rnx pack
2. $ rnx publish --token &lt;publisher-token&gt;
Publishing is currently limited to the Rovel Stars org while account
logins are being built. Scoped names look like @std/fs.</pre>
			<div class="mt-4 flex items-center gap-2 text-sm text-aura-muted">
				<GitBranch size={14} strokeWidth={1.5} />
				<span>Requires a reserved namespace handle.</span>
			</div>
			<button
				onclick={() => (publishOpen = false)}
				class="press mt-5 w-full rounded-md border border-aura-border px-4 py-2 text-sm hover:border-aura-borderHover"
			>
				Close
			</button>
		</div>
	</div>
{/if}
