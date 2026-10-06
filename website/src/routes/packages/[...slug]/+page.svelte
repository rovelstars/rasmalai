<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { renderMarkdown } from '$lib/docs/markdown';
	import AuraCode from '$lib/components/AuraCode.svelte';
	import ModuleDocs from '$lib/components/ModuleDocs.svelte';
	import type { DocModule } from '$lib/docs/api';
	import { shortDownloads, humanDate } from '$lib/packages-meta';
	import { untar, gunzip, isGzip } from '$lib/docs/untar';
import { importSnippet } from '$lib/docs/stdlib';

	let { data } = $props();
	let pkg = $derived(data.pkg);
	let activeVersion = $derived(data.active);

	interface StoredGuide {
		slug: string;
		title: string;
		html: string;
	}

	let guides = $derived.by((): StoredGuide[] => {
		try {
			const v = JSON.parse(activeVersion.guides ?? '[]') as unknown;
			if (!Array.isArray(v)) return [];
			return v.filter(
				(g): g is StoredGuide =>
					!!g && typeof (g as StoredGuide).slug === 'string' && typeof (g as StoredGuide).title === 'string'
			);
		} catch {
			return [];
		}
	});

	function normalizeModule(m: DocModule): DocModule {
		return {
			name: m.name,
			docs: m.docs ?? { description: '', tags: [] },
			functions: Array.isArray(m.functions) ? m.functions : [],
			classes: Array.isArray(m.classes) ? m.classes : [],
			enums: Array.isArray(m.enums) ? m.enums : [],
			constants: Array.isArray((m as DocModule).constants) ? m.constants : []
		};
	}

	let apiMods = $derived.by((): DocModule[] => {
		try {
			const v = JSON.parse(activeVersion.docJson ?? '{}') as { modules?: unknown };
			if (!v || !Array.isArray(v.modules)) return [];
			return (v.modules as DocModule[])
				.filter((m) => m && typeof m.name === 'string')
				.map(normalizeModule);
		} catch {
			return [];
		}
	});

	let openGuide = $state<string | null>(null);
	let selectedGuide = $derived(guides.find((g) => g.slug === openGuide) ?? guides[0] ?? null);

	type TabId = 'readme' | 'code' | 'guides' | 'api' | 'dependencies' | 'dependents' | 'versions';
	const TABS: TabId[] = ['readme', 'code', 'guides', 'api', 'dependencies', 'dependents', 'versions'];

	let tab = $derived.by((): TabId => {
		let t: TabId = 'readme';
		if (browser) {
			const q = page.url.searchParams.get('tab');
			if (q === 'code' || q === 'guides' || q === 'api' || q === 'dependencies' || q === 'dependents' || q === 'versions') t = q;
		}
		if (t === 'guides' && guides.length === 0) return 'readme';
		if (t === 'api' && apiMods.length === 0) return 'readme';
		return t;
	});

	let copied = $state(false);
	let tree = $state<Array<{ path: string; size: number }> | null>(null);
	let treeError = $state<string | null>(null);
	let openDirs = $state<string[]>([]);
	let openFile = $state<string | null>(null);
	let fileText = $state<string | null>(null);
	let fileSize = $state(0);
	let fileError = $state<string | null>(null);
	let fileLoading = $state(false);
	let dependents = $state<string[] | null>(null);
	let dependentsError = $state<string | null>(null);
	let manifestDeps = $state<Array<{ name: string; range: string }> | null>(null);
	let homepage = $state<string | null>(null);

	let readme = $derived(renderMarkdown(activeVersion.readme));

	function isoDate(ts: number): string {
		if (!ts) return '';
		return new Date(ts * 1000).toISOString().slice(0, 10);
	}

	function fmtSize(n: number): string {
		if (n >= 1024 * 1024) return (n / (1024 * 1024)).toFixed(1).replace(/\.0$/, '') + ' MiB';
		if (n >= 1024) return (n / 1024).toFixed(1).replace(/\.0$/, '') + ' KiB';
		return `${n} B`;
	}

	function avatarHue(name: string): number {
		let h = 0;
		for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
		return h % 360;
	}

	function nav(next: { tab?: string; v?: string }) {
		const t = next.tab ?? tab;
		const v = next.v ?? activeVersion.version;
		const base = v === pkg.latest ? `/packages/${pkg.name}` : `/packages/${pkg.name}@${v}`;
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

	let treeKey: string | null = null;
	let treeInflight = false;
	interface ChunkEntry {
		name: string;
		size: number;
		dir: boolean;
		chunks: string[];
	}
	let manifestEntries = $state<ChunkEntry[] | null>(null);
	let untarred = $state<Array<{ path: string; size: number; bytes: Uint8Array }> | null>(null);
	const chunkCache = new Map<string, Uint8Array>();

	async function fetchChunk(hash: string): Promise<Uint8Array> {
		const hit = chunkCache.get(hash);
		if (hit) return hit;
		const res = await fetch(`/api/packages/${pkg.name}@${activeVersion.version}/chunk/${hash}`);
		if (!res.ok) throw new Error(`chunk ${hash.slice(0, 12)} failed to load`);
		const bytes = new Uint8Array(await res.arrayBuffer());
		chunkCache.set(hash, bytes);
		return bytes;
	}

	async function assembleChunks(hashes: string[]): Promise<Uint8Array> {
		const parts = await Promise.all(hashes.map(fetchChunk));
		const total = parts.reduce((n, p) => n + p.length, 0);
		const out = new Uint8Array(total);
		let off = 0;
		for (const p of parts) {
			out.set(p, off);
			off += p.length;
		}
		return out;
	}

	async function loadTree() {
		const key = `${pkg.name}@${activeVersion.version}`;
		if (key === treeKey || treeInflight) return;
		treeInflight = true;
		manifestEntries = null;
		untarred = null;
		try {
			const res = await fetch(`/api/packages/${pkg.name}@${activeVersion.version}/chunks`);
			if (!res.ok) {
				treeError = res.status === 404 ? 'No file listing for this version yet.' : 'Could not load the file listing.';
				return;
			}
			const body = (await res.json()) as { entries?: unknown };
			manifestEntries = Array.isArray(body.entries)
				? (body.entries as ChunkEntry[]).filter((e) => e && typeof e.name === 'string')
				: [];
			const fallback = manifestEntries.length === 1 && manifestEntries[0].name === '' && !manifestEntries[0].dir;
			if (fallback) {
				const blob = await assembleChunks(manifestEntries[0].chunks);
				const raw = isGzip(blob) ? await gunzip(blob) : blob;
				untarred = untar(raw).map((f) => ({ path: f.path, size: f.size, bytes: f.bytes }));
				tree = untarred.map((f) => ({ path: f.path, size: f.size }));
			} else {
				tree = manifestEntries.filter((e) => !e.dir).map((e) => ({ path: e.name, size: e.size }));
			}
		} catch {
			treeError = 'Could not load the file listing.';
		} finally {
			treeKey = key;
			treeInflight = false;
		}
	}

	async function loadFile(path: string) {
		openFile = path;
		fileText = null;
		fileError = null;
		fileLoading = true;
		try {
			let bytes: Uint8Array | null = null;
			const local = untarred?.find((f) => f.path === path) ?? null;
			if (local) {
				bytes = local.bytes;
			} else {
				const entry = manifestEntries?.find((e) => !e.dir && e.name === path) ?? null;
				if (!entry) {
					fileError = 'File not found in this version.';
					return;
				}
				bytes = await assembleChunks(entry.chunks);
			}
			fileSize = bytes.length;
			if (isBinary(path) || bytes.length > 256 * 1024) {
				fileText = null;
				return;
			}
			try {
				fileText = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
			} catch {
				fileText = null;
			}
		} catch {
			fileError = 'Could not load the file.';
		} finally {
			fileLoading = false;
		}
	}

	let dependentsInflight = false;
	async function loadDependents() {
		if (dependents !== null || dependentsError !== null || dependentsInflight) return;
		dependentsInflight = true;
		try {
			const res = await fetch(`/api/packages/${pkg.name}/dependents`);
			if (!res.ok) {
				dependentsError = res.status === 404 ? 'No dependents data for this package yet.' : 'Could not load dependents.';
				return;
			}
			const body = (await res.json()) as { dependents?: Array<{ name: string }> };
			dependents = Array.isArray(body.dependents) ? body.dependents.map((d) => d.name) : [];
		} catch {
			dependentsError = 'Could not load dependents.';
		} finally {
			dependentsInflight = false;
		}
	}

	let manifestKey: string | null = null;
	let manifestInflight = false;
	async function loadManifest() {
		const key = `${pkg.name}@${activeVersion.version}`;
		if (key === manifestKey || manifestInflight) return;
		manifestInflight = true;
		if (pkg.dependencies.length > 0) {
			manifestDeps = pkg.dependencies.map((d) => ({ name: d, range: '' }));
			manifestKey = key;
			manifestInflight = false;
			return;
		}
		try {
			const res = await fetch(`/api/packages/${pkg.name}@${activeVersion.version}/manifest`);
			if (!res.ok) {
				manifestDeps = [];
				return;
			}
			const body = (await res.json()) as Record<string, unknown>;
			const deps = (body['deps'] ?? body['dependencies']) as unknown;
			if (deps && typeof deps === 'object' && !Array.isArray(deps)) {
				manifestDeps = Object.entries(deps as Record<string, unknown>).map(([name, range]) => ({
					name,
					range: typeof range === 'string' ? range : ''
				}));
			} else {
				manifestDeps = [];
			}
			const home = body['homepage'];
			if (typeof home === 'string' && home) homepage = home;
		} catch {
			manifestDeps = [];
		} finally {
			manifestKey = key;
			manifestInflight = false;
		}
	}

	onMount(() => {
		if (tab === 'code') loadTree();
		if (tab === 'dependents') loadDependents();
		if (tab === 'dependencies') loadManifest();
	});

	$effect(() => {
		if (!browser) return;
		if (tab === 'code') loadTree();
		if (tab === 'dependents') loadDependents();
		if (tab === 'dependencies') loadManifest();
	});



	function toggleDir(dir: string) {
		openDirs = openDirs.includes(dir) ? openDirs.filter((d) => d !== dir) : [...openDirs, dir];
	}

	function isBinary(path: string): boolean {
		return /\.(png|jpe?g|gif|webp|ico|bmp|pdf|zip|tar|gz|wasm|so|dylib|dll|exe|o|a|woff2?|ttf|otf|mp3|mp4)$/i.test(path);
	}

	type TreeNode = { name: string; full: string; size: number; kids: Map<string, TreeNode> };

	let fileTree = $derived.by(() => {
		const root: TreeNode = { name: '', full: '', size: 0, kids: new Map() };
		for (const f of tree ?? []) {
			const parts = f.path.split('/').filter(Boolean);
			let node = root;
			for (let i = 0; i < parts.length; i++) {
				const part = parts[i];
				const full = parts.slice(0, i + 1).join('/');
				let kid = node.kids.get(part);
				if (!kid) {
					kid = { name: part, full, size: 0, kids: new Map() };
					node.kids.set(part, kid);
				}
				node = kid;
				if (i === parts.length - 1) node.size = f.size;
			}
		}
		const sortKids = (n: TreeNode) => {
			n.kids = new Map(
				[...n.kids.entries()].sort((a, b) => {
					const aDir = a[1].kids.size > 0 ? 0 : 1;
					const bDir = b[1].kids.size > 0 ? 0 : 1;
					return aDir - bDir || a[0].localeCompare(b[0]);
				})
			);
			for (const kid of n.kids.values()) sortKids(kid);
		};
		sortKids(root);
		return root;
	});
</script>

<svelte:head>
	<title>{pkg.name} v{activeVersion.version} — Packages</title>
	<meta name="description" content={pkg.description} />
	<link rel="canonical" href="https://rasmalai.rovelstars.com/packages/{pkg.name}" />
</svelte:head>

<main class="mx-auto max-w-7xl px-4 pb-16">
	<nav class="mt-8 font-mono text-xs text-aura-muted" aria-label="Breadcrumb">
		<a href="/packages" class="hover:text-aura-text">Packages</a>
		<span class="mx-1.5" aria-hidden="true">/</span>
		<span class="text-aura-text">{pkg.name}</span>
	</nav>

	<div class="panel mt-4 p-5">
		<div class="flex flex-wrap items-baseline gap-x-3 gap-y-2">
			<span class="font-mono text-lg font-bold text-aura-text">{pkg.name}</span>
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
				<span class="rounded bg-aura-surfaceElevated px-1.5 py-0.5 font-mono text-[11px] text-aura-muted"
					>v{activeVersion.version}</span
				>
			{/if}
			<span class="rounded border border-aura-border px-1.5 py-0.5 font-mono text-[11px] text-aura-muted"
				>{pkg.license}</span
			>
			<span class="tabular ml-auto font-mono text-xs text-aura-muted"
				>{shortDownloads(pkg.downloads)} downloads</span
			>
		</div>
		<p class="mt-2 text-sm text-aura-muted">{pkg.description}</p>
		<p class="mt-2 flex items-center gap-1.5 font-mono text-xs text-aura-muted">
			<span
				class="flex h-5 w-5 items-center justify-center rounded-full text-[11px] font-bold text-white"
				style="background-color: hsl({avatarHue(pkg.author)} 45% 38%)"
				aria-hidden="true">{(pkg.author.replace(/^@/, '')[0] ?? '?').toUpperCase()}</span
			>
			<span>by <span class="text-aura-text">@{pkg.author.replace(/^@/, '')}</span></span>
		</p>
		{#if pkg.name.startsWith('@std/')}
			<p class="mt-2 text-sm text-aura-muted"><code class="font-mono text-[13px]">{importSnippet(pkg.name.slice('@std/'.length))}</code></p>
		{/if}
		{#if pkg.name === '@std/prelude'}
			<p class="mt-2 rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm">
				<span class="font-mono text-[11px] uppercase tracking-wider text-emerald-400">implicit scope</span><br />
				Symbols in <code class="font-mono text-[13px]">@std/prelude</code> are available in every Rasmalai
				source file out-of-the-box without an explicit import. Explicit imports are supported for
				disambiguation.
			</p>
		{/if}
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

	<div class="mt-4 flex gap-1 overflow-x-auto border-b border-aura-border" role="tablist" aria-label="Package views">
		{#each TABS as t}
			{#if (t !== 'guides' || guides.length > 0) && (t !== 'api' || apiMods.length > 0)}
			<button
				role="tab"
				aria-selected={tab === t}
				onclick={() => nav({ tab: t })}
				class="press -mb-px shrink-0 border-b-2 px-4 py-2 font-mono text-sm capitalize transition-colors duration-150 {tab === t
					? 'border-aura-purple text-aura-text'
					: 'border-transparent text-aura-muted hover:text-aura-text'}"
			>
				{t}
			</button>
			{/if}
		{/each}
	</div>

	<div class="grid gap-8 lg:grid-cols-[minmax(0,1fr)_240px]">
		<div class="min-w-0 pt-4">
			{#if tab === 'readme'}
				<article class="doc-prose">
					{@html readme.html}
				</article>
			{:else if tab === 'code'}
				{#if treeError}
					<p class="panel p-6 text-center text-sm text-aura-muted">{treeError}</p>
				{:else if tree === null}
					<p class="panel p-6 text-center font-mono text-xs text-aura-muted" aria-live="polite">Loading file list…</p>
				{:else if tree.length === 0}
					<p class="panel p-6 text-center text-sm text-aura-muted">This version ships no browsable files.</p>
				{:else}
					<div class="grid gap-4 xl:grid-cols-[280px_minmax(0,1fr)]">
						<div class="panel max-h-[560px] overflow-auto p-2" aria-label="File tree">
							{#snippet nodes(n: TreeNode, depth: number)}
								{#each [...n.kids.values()] as kid}
									{#if kid.kids.size > 0}
										<div>
											<button
												onclick={() => toggleDir(kid.full)}
												class="flex w-full items-center gap-1.5 rounded px-2 py-1 text-left font-mono text-[13px] text-aura-muted hover:bg-aura-surfaceElevated hover:text-aura-text"
												style="padding-left: {8 + depth * 12}px"
												aria-expanded={openDirs.includes(kid.full)}
											>
												<span class="inline-block w-3 text-aura-purple" aria-hidden="true"
													>{openDirs.includes(kid.full) ? '▾' : '▸'}</span
												>
												<span class="truncate">{kid.name}/</span>
											</button>
											{#if openDirs.includes(kid.full)}
												{@render nodes(kid, depth + 1)}
											{/if}
										</div>
									{:else}
										<button
											onclick={() => loadFile(kid.full)}
											class="flex w-full items-center gap-1.5 rounded px-2 py-1 text-left font-mono text-[13px] {openFile === kid.full
												? 'bg-aura-surfaceElevated text-aura-text'
												: 'text-aura-muted hover:bg-aura-surfaceElevated hover:text-aura-text'}"
											style="padding-left: {8 + depth * 12 + 12}px"
										>
											<span class="min-w-0 flex-1 truncate">{kid.name}</span>
											<span class="tabular shrink-0 text-[11px] opacity-70">{fmtSize(kid.size)}</span>
										</button>
									{/if}
								{/each}
							{/snippet}
							{@render nodes(fileTree, 0)}
						</div>
						<div class="panel min-w-0 overflow-hidden" aria-live="polite">
							{#if fileLoading}
								<p class="p-6 text-center font-mono text-xs text-aura-muted">Loading {openFile}…</p>
							{:else if fileError}
								<p class="p-6 text-center text-sm text-aura-muted">{fileError}</p>
							{:else if openFile === null}
								<p class="p-6 text-center text-sm text-aura-muted">Pick a file to read it here.</p>
							{:else if fileText === null || isBinary(openFile)}
								<div class="p-6 text-center text-sm text-aura-muted">
									<p class="font-mono text-xs">{openFile} — {fmtSize(fileSize)} of binary data.</p>
									<a
										href="/api/packages/{pkg.name}@{activeVersion.version}/download"
										class="mt-3 inline-block rounded border border-aura-border px-3 py-1.5 font-mono text-xs text-aura-cyan hover:border-aura-borderHover"
										download>download tarball</a
									>
								</div>
							{:else}
								<p class="border-b border-aura-border px-4 py-2 font-mono text-xs text-aura-muted">{openFile}</p>
								<AuraCode source={fileText} showCopy />
							{/if}
						</div>
					</div>
				{/if}
			{:else if tab === 'guides'}
				{#if guides.length === 0}
					<p class="panel p-6 text-center text-sm text-aura-muted">This version ships no guides.</p>
				{:else}
					<div class="flex flex-wrap gap-2" role="tablist" aria-label="Guides">
						{#each guides as g}
							<button
								role="tab"
								aria-selected={selectedGuide?.slug === g.slug}
								onclick={() => (openGuide = g.slug)}
								class="press rounded border px-3 py-1.5 font-mono text-[13px] {selectedGuide?.slug === g.slug
									? 'border-aura-purple text-aura-text'
									: 'border-aura-border text-aura-muted hover:border-aura-borderHover hover:text-aura-text'}"
							>
								{g.title}
							</button>
						{/each}
					</div>
					{#if selectedGuide}
						<article class="doc-prose mt-4">
							{@html selectedGuide.html}
						</article>
					{/if}
				{/if}
			{:else if tab === 'api'}
				{#if apiMods.length === 0}
					<p class="panel p-6 text-center text-sm text-aura-muted">No API reference published for this version yet.</p>
				{:else}
					{#each apiMods as m (m.name)}
						<h2 class="mt-6 font-mono text-sm font-bold text-aura-text first:mt-0">{m.name}</h2>
						<ModuleDocs mod={m} allMods={apiMods} idPrefix="{m.name}-" />
					{/each}
				{/if}
			{:else if tab === 'dependencies'}
				{#if manifestDeps === null}
					<p class="panel p-6 text-center font-mono text-xs text-aura-muted" aria-live="polite">Loading dependencies…</p>
				{:else if manifestDeps.length === 0}
					<p class="panel p-6 text-center text-sm text-aura-muted">No dependencies.</p>
				{:else}
					<ul class="panel divide-y divide-aura-border">
						{#each manifestDeps as d}
							<li class="flex items-baseline gap-3 px-4 py-2.5">
								<a href="/packages/{d.name}" class="font-mono text-sm font-bold text-aura-cyan hover:underline">{d.name}</a>
								{#if d.range}<span class="tabular font-mono text-xs text-aura-muted">{d.range}</span>{/if}
							</li>
						{/each}
					</ul>
				{/if}
			{:else if tab === 'dependents'}
				{#if dependentsError}
					<p class="panel p-6 text-center text-sm text-aura-muted">{dependentsError}</p>
				{:else if dependents === null}
					<p class="panel p-6 text-center font-mono text-xs text-aura-muted" aria-live="polite">Loading dependents…</p>
				{:else if dependents.length === 0}
					<p class="panel p-6 text-center text-sm text-aura-muted">No known dependents.</p>
				{:else}
					<ul class="panel divide-y divide-aura-border">
						{#each dependents as name}
							<li class="px-4 py-2.5">
								<a href="/packages/{name}" class="font-mono text-sm font-bold text-aura-cyan hover:underline">{name}</a>
							</li>
						{/each}
					</ul>
				{/if}
			{:else}
				<div class="panel overflow-x-auto">
					<table class="w-full min-w-[560px] border-collapse font-mono text-[13px]">
						<thead>
							<tr class="border-b border-aura-border text-left text-aura-muted">
								<th class="px-4 py-2.5 font-medium">version</th>
								<th class="px-4 py-2.5 font-medium">released</th>
								<th class="px-4 py-2.5 font-medium">sha-256</th>
								<th class="px-4 py-2.5 text-right font-medium">view</th>
							</tr>
						</thead>
						<tbody>
							{#each pkg.versions as v}
								<tr class="border-b border-aura-border last:border-0">
									<td class="px-4 py-2.5 text-aura-purple">v{v.version}</td>
									<td class="tabular px-4 py-2.5 text-aura-muted" title={isoDate(v.createdAt)}>{humanDate(v.createdAt)}</td>
									<td class="tabular px-4 py-2.5 text-aura-muted">{v.checksum.slice(0, 12)}...</td>
									<td class="px-4 py-2.5 text-right">
										<button onclick={() => nav({ v: v.version })} class="text-aura-cyan hover:underline">view</button>
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
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">install</p>
				<div class="mt-2 flex items-center gap-2">
					<code class="tabular min-w-0 flex-1 overflow-x-auto font-mono text-[13px] text-aura-cyan">rnx add {pkg.name}</code>
					<button
						onclick={copyInstall}
						class="shrink-0 rounded border border-aura-border px-2 py-0.5 font-mono text-[11px] text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
					>
						{copied ? 'copied' : 'copy'}
					</button>
				</div>
			</div>
			<div class="panel p-4">
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">links</p>
				<ul class="mt-1 space-y-1 font-mono text-[13px]">
					{#if pkg.name.startsWith('@std/')}
						<li><a href="/docs/@std/{pkg.name.slice('@std/'.length)}/overview" class="break-all text-aura-cyan hover:underline">guides</a></li>
						<li><a href="/docs/@std/{pkg.name.slice('@std/'.length)}/api" class="break-all text-aura-cyan hover:underline">api reference</a></li>
					{:else}
						{#if guides.length > 0}
							<li><button onclick={() => nav({ tab: 'guides' })} class="break-all text-aura-cyan hover:underline">guides</button></li>
						{/if}
						{#if apiMods.length > 0}
							<li><button onclick={() => nav({ tab: 'api' })} class="break-all text-aura-cyan hover:underline">api reference</button></li>
						{/if}
					{/if}
				{#if pkg.repository}
						<li><a href={pkg.repository} rel="external" class="break-all text-aura-cyan hover:underline">repository</a></li>
					{/if}
					{#if homepage}
						<li><a href={homepage} rel="external" class="break-all text-aura-cyan hover:underline">homepage</a></li>
					{/if}
					{#if !pkg.repository && !homepage}
						<li class="text-aura-muted">No links listed.</li>
					{/if}
				</ul>
			</div>
			<div class="panel space-y-2 p-4">
				<p class="font-mono text-[11px] uppercase tracking-wider text-aura-muted">details</p>
				<dl class="space-y-1.5 text-[13px]">
					<div class="flex justify-between gap-2">
						<dt class="text-aura-muted">version</dt>
						<dd class="tabular font-mono">v{activeVersion.version}</dd>
					</div>
					<div class="flex justify-between gap-2">
						<dt class="text-aura-muted">license</dt>
						<dd class="font-mono">{pkg.license}</dd>
					</div>
					<div class="flex justify-between gap-2">
						<dt class="text-aura-muted">published</dt>
						<dd class="tabular font-mono" title={isoDate(pkg.versions[0]?.createdAt ?? 0)}>
							{humanDate(pkg.versions[0]?.createdAt ?? 0)}
						</dd>
					</div>
					<div class="flex justify-between gap-2">
						<dt class="text-aura-muted">downloads</dt>
						<dd class="tabular font-mono">{shortDownloads(pkg.downloads)}</dd>
					</div>
					<div class="flex justify-between gap-2">
						<dt class="text-aura-muted">versions</dt>
						<dd class="tabular font-mono">{pkg.versions.length}</dd>
					</div>
				</dl>
			</div>
		</aside>
	</div>
</main>
