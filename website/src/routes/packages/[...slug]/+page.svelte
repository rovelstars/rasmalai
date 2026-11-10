<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { renderMarkdown } from '$lib/docs/markdown';
	import { canonicalUrl } from '$lib/site';
	import AuraCode from '$lib/components/AuraCode.svelte';
	import ModuleDocs from '$lib/components/ModuleDocs.svelte';
	import type { DocModule } from '$lib/docs/api';
	import { displayModuleName, isGeneratedModule } from '$lib/docs/api';
	import { humanDate } from '$lib/packages-meta';
import { PlaygroundRunner } from '$lib/playground/runner';
import { isFormattablePackagePath, selectPackageText } from '$lib/packages-format';
	import { untar, gunzip, isGzip } from '$lib/docs/untar';
	import '$lib/styles/registry.css';

	let { data } = $props();
	let pkg = $derived(data.pkg);
	let activeVersion = $derived(data.active);

	interface StoredGuide {
		slug: string;
		title: string;
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

	type TabId = 'readme' | 'code' | 'api' | 'dependencies' | 'dependents' | 'versions';
	const TABS: TabId[] = ['readme', 'code', 'api', 'dependencies', 'dependents', 'versions'];
	const TAB_LABELS: Record<TabId, string> = {
		readme: 'Readme',
		code: 'Files',
		api: 'API',
		dependencies: 'Dependencies',
		dependents: 'Dependents',
		versions: 'Versions'
	};

	const visibleTabs = $derived(TABS.filter((t) => t !== 'api' || apiMods.length > 0));

	let tab = $derived.by((): TabId => {
		let t: TabId = 'readme';
		if (browser) {
			const q = page.url.searchParams.get('tab');
			if (q === 'code' || q === 'api' || q === 'dependencies' || q === 'dependents' || q === 'versions') t = q;
		}
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
	let fileNotice = $state<string | null>(null);
	let fileRequest = 0;
	let formatRunner: PlaygroundRunner | null = null;
	let dependents = $state<string[] | null>(null);
	let dependentsError = $state<string | null>(null);
	let manifestDeps = $state<Array<{ name: string; range: string }> | null>(null);
	let homepage = $state<string | null>(null);

	let permissions = $derived(activeVersion.permissions ?? []);

	let readme = $derived.by(() => {
		const rendered = renderMarkdown(activeVersion.readme);
		// The page header already names the package, so drop a leading readme H1.
		return { ...rendered, html: rendered.html.replace(/^\s*<h1[^>]*>[\s\S]*?<\/h1>/, '') };
	});

	function isoDate(ts: number): string {
		if (!ts) return '';
		return new Date(ts * 1000).toISOString().slice(0, 10);
	}

	function fmtSize(n: number): string {
		if (n >= 1024 * 1024) return (n / (1024 * 1024)).toFixed(1).replace(/\.0$/, '') + ' MiB';
		if (n >= 1024) return (n / 1024).toFixed(1).replace(/\.0$/, '') + ' KiB';
		return `${n} B`;
	}

	function nav(next: { tab?: string; v?: string }) {
		const t = next.tab ?? tab;
		const v = next.v ?? activeVersion.version;
		const base = v === pkg.latest ? `/packages/${pkg.name}` : `/packages/${pkg.name}@${v}`;
		goto(`${base}?tab=${t}`, { keepFocus: true });
	}

	function nextTabIndex(event: KeyboardEvent, length: number, index: number): number | null {
		const last = length - 1;
		switch (event.key) {
			case 'ArrowRight':
			case 'ArrowDown':
				return index === last ? 0 : index + 1;
			case 'ArrowLeft':
			case 'ArrowUp':
				return index === 0 ? last : index - 1;
			case 'Home':
				return 0;
			case 'End':
				return last;
			default:
				return null;
		}
	}

	function focusTab(event: KeyboardEvent, index: number) {
		const tabs = (event.currentTarget as HTMLElement)
			.closest('[role="tablist"]')
			?.querySelectorAll<HTMLButtonElement>('[role="tab"]');
		tabs?.[index]?.focus();
	}

	function onSpineKeydown(event: KeyboardEvent, id: TabId) {
		const index = visibleTabs.indexOf(id);
		const next = nextTabIndex(event, visibleTabs.length, index);
		if (next === null) return;
		event.preventDefault();
		focusTab(event, next);
		nav({ tab: visibleTabs[next] });
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
	let treeInflightKey: string | null = null;
	let treeEpoch = 0;
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
		const res = await fetch(`/api/v0/packages/${pkg.name}@${activeVersion.version}/chunk/${hash}`);
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
		if (key === treeKey || key === treeInflightKey) return;
		const epoch = treeEpoch;
		const reopen = openFile;
		treeInflightKey = key;
		treeError = null;
		manifestEntries = null;
		untarred = null;
		try {
			const res = await fetch(`/api/v0/packages/${pkg.name}@${activeVersion.version}/chunks`);
			if (epoch !== treeEpoch) return;
			if (!res.ok) {
				treeError = res.status === 404 ? 'No file listing for this version yet.' : 'Could not load the file listing.';
				openFile = null;
				fileText = null;
				fileSize = 0;
				fileLoading = false;
				return;
			}
			const body = (await res.json()) as { entries?: unknown };
			if (epoch !== treeEpoch) return;
			manifestEntries = Array.isArray(body.entries)
				? (body.entries as ChunkEntry[]).filter((e) => e && typeof e.name === 'string')
				: [];
			const fallback = manifestEntries.length === 1 && manifestEntries[0].name === '' && !manifestEntries[0].dir;
			let nextTree: Array<{ path: string; size: number }>;
			if (fallback) {
				const blob = await assembleChunks(manifestEntries[0].chunks);
				if (epoch !== treeEpoch) return;
				const raw = isGzip(blob) ? await gunzip(blob) : blob;
				if (epoch !== treeEpoch) return;
				untarred = untar(raw).map((f) => ({ path: f.path, size: f.size, bytes: f.bytes }));
				nextTree = untarred.map((f) => ({ path: f.path, size: f.size }));
			} else {
				nextTree = manifestEntries.filter((e) => !e.dir).map((e) => ({ path: e.name, size: e.size }));
			}
			tree = nextTree;
			if (reopen && nextTree.some((f) => f.path === reopen)) {
				void loadFile(reopen);
			} else {
				openFile = null;
				fileText = null;
				fileSize = 0;
				fileLoading = false;
			}
		} catch {
			if (epoch === treeEpoch) {
				treeError = 'Could not load the file listing.';
				openFile = null;
				fileText = null;
				fileSize = 0;
				fileLoading = false;
			}
		} finally {
			if (epoch === treeEpoch && treeInflightKey === key) {
				treeKey = key;
				treeInflightKey = null;
			}
		}
	}

	async function loadFile(path: string) {
		const request = ++fileRequest;
		openFile = path;
		fileText = null;
		fileError = null;
		fileNotice = null;
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
			if (request !== fileRequest) return;
			fileSize = bytes.length;
			if (isBinary(path) || bytes.length > 256 * 1024) {
				fileText = null;
				return;
			}
			let raw: string;
			try {
				raw = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
			} catch {
				fileText = null;
				return;
			}
			if (!isFormattablePackagePath(path)) {
				fileText = raw;
				return;
			}
			let formatRaw: string | null = null;
			try {
				formatRunner ??= new PlaygroundRunner();
				formatRaw = await formatRunner.format(raw);
			} catch {
				formatRaw = null;
			}
			if (request !== fileRequest) return;
			const outcome = selectPackageText(raw, path, formatRaw);
			fileText = outcome.text;
			fileNotice = outcome.notice;
		} catch {
			if (request === fileRequest) fileError = 'Could not load the file.';
		} finally {
			if (request === fileRequest) fileLoading = false;
		}
	}

	let dependentsInflight = false;
	async function loadDependents() {
		if (dependents !== null || dependentsError !== null || dependentsInflight) return;
		dependentsInflight = true;
		try {
			const res = await fetch(`/api/v0/packages/${pkg.name}/dependents`);
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
	let manifestInflightKey: string | null = null;
	let manifestEpoch = 0;
	async function loadManifest() {
		const key = `${pkg.name}@${activeVersion.version}`;
		if (key === manifestKey || key === manifestInflightKey) return;
		const epoch = manifestEpoch;
		manifestInflightKey = key;
		manifestDeps = null;
		homepage = null;
		if (pkg.dependencies.length > 0) {
			manifestDeps = pkg.dependencies.map((d) => ({ name: d, range: '' }));
			manifestKey = key;
			manifestInflightKey = null;
			return;
		}
		try {
			const res = await fetch(`/api/v0/packages/${pkg.name}@${activeVersion.version}/manifest`);
			if (epoch !== manifestEpoch) return;
			if (!res.ok) {
				manifestDeps = [];
				return;
			}
			const body = (await res.json()) as Record<string, unknown>;
			if (epoch !== manifestEpoch) return;
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
			if (epoch === manifestEpoch) manifestDeps = [];
		} finally {
			if (epoch === manifestEpoch && manifestInflightKey === key) {
				manifestKey = key;
				manifestInflightKey = null;
			}
		}
	}

	onMount(() => {
		if (tab === 'code') loadTree();
		if (tab === 'dependents') loadDependents();
		if (tab === 'dependencies') loadManifest();
	});

	let scopeKey: string | null = null;
	$effect(() => {
		const key = `${pkg.name}@${activeVersion.version}`;
		if (key === scopeKey) return;
		const initial = scopeKey === null;
		scopeKey = key;
		if (initial) return;
		// A version switch invalidates in-flight reads; the epochs keep stale
		// tree, manifest and file responses from landing in the new version.
		treeEpoch++;
		tree = null;
		treeError = null;
		treeKey = null;
		treeInflightKey = null;
		openDirs = [];
		fileText = null;
		fileSize = 0;
		fileError = null;
		fileNotice = null;
		fileLoading = openFile !== null;
		fileRequest++;
		manifestEpoch++;
		manifestDeps = null;
		manifestKey = null;
		manifestInflightKey = null;
		homepage = null;
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
			if (parts.some((p) => p.startsWith('.'))) continue;
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
	<title>{pkg.name} v{activeVersion.version} | Packages</title>
	<meta name="description" content={pkg.description || `${pkg.name} package for Rasmalai.`} />
	<link rel="canonical" href={canonicalUrl(`/packages/${pkg.name}`)} />
</svelte:head>

<main class="shell pb-16">
	<div class="docs-bar">
		<nav class="docs-crumb" aria-label="Breadcrumb">
			<a href="/packages">Packages</a>
		</nav>
	</div>

	<header class="dossier-mast">
		<div class="dossier-head">
			<h1 class="dossier-title">{pkg.name}</h1>
			{#if pkg.description}
				<p class="dossier-sub">{pkg.description}</p>
			{/if}
			<dl class="dossier-facts">
				<div>
					<dt>Author</dt>
					<dd>{pkg.author}</dd>
				</div>
				<div>
					<dt>License</dt>
					<dd>{pkg.license}</dd>
				</div>
				<div>
					<dt>Published</dt>
					<dd>{humanDate(pkg.versions[0]?.createdAt ?? 0)}</dd>
				</div>
				<div>
					<dt>Versions</dt>
					<dd>{pkg.versions.length}</dd>
				</div>
			</dl>
		</div>

		<div class="dossier-install">
			<p class="dossier-install-label">Install</p>
			<div class="dossier-install-line">
				<code>rnx add {pkg.name}</code>
				<button onclick={copyInstall} class="dossier-copy" style="white-space: nowrap" aria-label="Copy install command">
					{copied ? 'copied' : 'copy'}
				</button>
			</div>
			{#if pkg.versions.length > 1}
				<label class="dossier-version">
					<span>Version</span>
					<select
						value={activeVersion.version}
						onchange={(e) => nav({ v: (e.target as HTMLSelectElement).value })}
						aria-label="Select version"
					>
						{#each pkg.versions as v}
							<option value={v.version}>v{v.version}</option>
						{/each}
					</select>
				</label>
			{/if}
		</div>
	</header>

	{#if pkg.name === '@std/prelude'}
		<div class="callout callout-tip dossier-callout">
			<p class="callout-label">Implicit scope</p>
			<p>
				Symbols in <code>@std/prelude</code> are available in every Rasmalai source file without an
				explicit import. Explicit imports are supported for disambiguation.
			</p>
		</div>
	{/if}

	<div class="dossier-body">
		<aside class="dossier-spine">
			<div class="dossier-tabs" role="tablist" aria-label="Package views">
				{#each visibleTabs as t}
					<button
						id={`pkg-view-tab-${t}`}
						role="tab"
						aria-selected={tab === t}
						aria-controls="pkg-view-panel"
						tabindex={tab === t ? 0 : -1}
						onclick={() => nav({ tab: t })}
						onkeydown={(event) => onSpineKeydown(event, t)}
						class="dossier-tab"
					>
						{TAB_LABELS[t]}
					</button>
				{/each}
			</div>

			<div class="dossier-block">
				<p class="dossier-label">Links</p>
				<div class="dossier-link-list">
					{#if pkg.name.startsWith('@std/')}
						<a href="/docs/@std/{pkg.name.slice('@std/'.length)}/overview" class="dossier-link"
							>guides</a
						>
						<a href="/docs/@std/{pkg.name.slice('@std/'.length)}/api" class="dossier-link"
							>api reference</a
						>
					{:else}
						{#if guides.length > 0}
							<a href={`/docs/${pkg.name}/${guides[0].slug}`} class="dossier-link">guides</a>
						{/if}
						{#if apiMods.length > 0}
							<button onclick={() => nav({ tab: 'api' })} class="dossier-link">api reference</button>
						{/if}
					{/if}
					{#if pkg.repository}
						<a href={pkg.repository} rel="external" class="dossier-link">repository</a>
					{/if}
					{#if homepage}
						<a href={homepage} rel="external" class="dossier-link">homepage</a>
					{/if}
					{#if !pkg.repository && !homepage && !pkg.name.startsWith('@std/') && guides.length === 0 && apiMods.length === 0}
						<p class="dossier-link dossier-link-empty">No links listed.</p>
					{/if}
				</div>
			</div>

			{#if permissions.length > 0}
				<div class="dossier-block">
					<p class="dossier-label">Permissions</p>
					<ul class="dossier-perms">
						{#each permissions as p}
							<li>
								<p class="dossier-perm" title={p.arg ? `${p.domain}:${p.arg}` : p.domain}>
									{p.domain}{#if p.arg}<span>:{p.arg}</span>{/if}
								</p>
								{#if p.reason}
									<p class="dossier-perm-reason">{p.reason}</p>
								{:else}
									<p class="dossier-perm-reason dossier-perm-empty">no reason given</p>
								{/if}
							</li>
						{/each}
					</ul>
				</div>
			{/if}
		</aside>

		<div
			class="dossier-content"
			id="pkg-view-panel"
			role="tabpanel"
			aria-labelledby={`pkg-view-tab-${tab}`}
		>
			{#if tab === 'readme'}
				<article class="doc-prose">
					<!-- eslint-disable-next-line svelte/no-at-html-tags -->
					{@html readme.html}
				</article>
			{:else if tab === 'code'}
				{#if treeError}
					<p class="py-6 text-sm text-aura-muted">{treeError}</p>
				{:else if tree === null}
					<p class="py-6 text-xs text-aura-muted" aria-live="polite">Loading file list…</p>
				{:else if tree.length === 0}
					<p class="py-6 text-sm text-aura-muted">This version ships no browsable files.</p>
				{:else}
					<div class="grid gap-4 xl:grid-cols-[280px_minmax(0,1fr)]">
						<div class="file-tree" aria-label="File tree">
							{#snippet nodes(n: TreeNode, depth: number)}
								{#each Array.from(n.kids.values()) as kid}
									{#if kid.kids.size > 0}
										<div>
											<button
												onclick={() => toggleDir(kid.full)}
												class="file-node whitespace-nowrap"
												style="padding-left: {8 + depth * 12}px; white-space: nowrap"
												aria-expanded={openDirs.includes(kid.full)}
											>
												<span class="file-caret" aria-hidden="true"
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
											class="file-node whitespace-nowrap"
											class:file-node-active={openFile === kid.full}
											style="padding-left: {8 + depth * 12 + 12}px; white-space: nowrap"
										>
											<span class="min-w-0 flex-1 truncate">{kid.name}</span>
											<span class="tabular shrink-0 text-[10px] opacity-70">{fmtSize(kid.size)}</span>
										</button>
									{/if}
								{/each}
							{/snippet}
							{@render nodes(fileTree, 0)}
						</div>
						<div class="file-view" aria-live="polite">
							{#if fileLoading}
								<p class="p-6 text-center text-xs text-aura-muted">Loading {openFile}…</p>
							{:else if fileError}
								<p class="p-6 text-center text-sm text-aura-muted">{fileError}</p>
							{:else if openFile === null}
								<p class="p-6 text-center text-sm text-aura-muted">Pick a file to read it here.</p>
							{:else if fileText === null || isBinary(openFile)}
								<div class="p-6 text-center text-sm text-aura-muted">
									<p class="text-xs"><span class="font-mono">{openFile}</span> - {fmtSize(fileSize)} of binary data.</p>
								</div>
							{:else}
								<p class="file-view-head">{openFile}</p>
								{#if fileNotice}
									<p class="file-view-note">{fileNotice}</p>
								{/if}
								<AuraCode source={fileText} showCopy />
							{/if}
						</div>
					</div>
				{/if}
			{:else if tab === 'api'}
				{#if apiMods.length === 0}
					<p class="py-6 text-sm text-aura-muted">No API reference published for this version yet.</p>
				{:else}
					{#each apiMods as m (m.name)}
						<h2 class="pkg-api-mod">
							{displayModuleName(m.name)}
							{#if isGeneratedModule(m.name)}
								<span class="text-[11px] font-normal text-aura-faint">generated</span>
							{/if}
						</h2>
						<ModuleDocs mod={m} allMods={apiMods} idPrefix="{m.name}-" />
					{/each}
				{/if}
			{:else if tab === 'dependencies'}
				{#if manifestDeps === null}
					<p class="py-6 text-xs text-aura-muted" aria-live="polite">Loading dependencies…</p>
				{:else if manifestDeps.length === 0}
					<p class="py-6 text-sm text-aura-muted">No dependencies.</p>
				{:else}
					<ul class="index-list">
						{#each manifestDeps as d}
							<li>
								<a href="/packages/{d.name}" class="index-row">
									<span class="index-row-num">-</span>
									<span class="index-row-title font-mono">{d.name}</span>
									<span class="index-row-desc font-mono">{d.range}</span>
								</a>
							</li>
						{/each}
					</ul>
				{/if}
			{:else if tab === 'dependents'}
				{#if dependentsError}
					<p class="py-6 text-sm text-aura-muted">{dependentsError}</p>
				{:else if dependents === null}
					<p class="py-6 text-xs text-aura-muted" aria-live="polite">Loading dependents…</p>
				{:else if dependents.length === 0}
					<p class="py-6 text-sm text-aura-muted">No known dependents.</p>
				{:else}
					<ul class="index-list">
						{#each dependents as name}
							<li>
								<a href="/packages/{name}" class="index-row">
									<span class="index-row-num">-</span>
									<span class="index-row-title font-mono">{name}</span>
									<span class="index-row-desc"></span>
								</a>
							</li>
						{/each}
					</ul>
				{/if}
			{:else}
				<div class="doc-table-wrap mt-0">
					<table class="doc-table">
						<thead>
							<tr>
								<th>version</th>
								<th>released</th>
								<th>sha-256</th>
								<th class="text-right">view</th>
							</tr>
						</thead>
						<tbody>
							{#each pkg.versions as v}
								<tr>
									<td class="font-mono text-aura-accent">v{v.version}</td>
									<td class="tabular font-mono text-aura-muted" title={isoDate(v.createdAt)}
										>{humanDate(v.createdAt)}</td
									>
									<td class="tabular font-mono text-aura-muted">{v.checksum.slice(0, 12)}…</td>
									<td class="text-right">
										<button onclick={() => nav({ v: v.version })} class="doc-btn ml-auto">view</button>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</div>

	</div>
</main>

<style>
	.pkg-api-mod {
		overflow-wrap: anywhere;
	}

	:where(button, a, select):focus-visible {
		outline: 2px solid var(--color-accent);
		outline-offset: 2px;
	}
</style>
