<script lang="ts">
	import { onMount } from 'svelte';
	import { Search } from 'lucide-svelte';
	import { goto } from '$app/navigation';
	import { searchEntries, type SearchEntry, type ScoredEntry } from '$lib/docs/search';
	import { fetchDataFile } from '$lib/docs/data-url';

	// search-index.json is generated at build time (prebuild) into
	// static/data and served same-origin. This export is the legacy
	// unversioned fallback; resolution prefers the deploy-versioned copy
	// via /data/version.json (see data-url.ts), so no build carries a
	// snapshot. Point a deploy somewhere else by editing the fallback here.
	export const searchIndexUrl = '/data/search-index.json';

	interface Badge {
		label: string;
		class: string;
	}

	const BADGES: Record<SearchEntry['category'], Badge> = {
		manual: { label: 'Manual', class: 'text-aura-green' },
		guide: { label: 'Guide', class: 'text-aura-cyan' },
		package: { label: 'Package', class: 'text-aura-purple' },
		symbol: { label: 'API', class: 'text-aura-orange' }
	};

	let index: SearchEntry[] = $state([]);
	let indexMissing = $state(false);

	let open = $state(false);
	let query = $state('');
	let selected = $state(0);
	let input: HTMLInputElement | null = $state(null);
	let list: HTMLUListElement | null = $state(null);

	// The navbar carries backdrop-blur, which traps `fixed` descendants
	// inside the header box. Teleport the modal to <body> so the overlay
	// covers the viewport instead of rendering as a bar.
	function portal(node: HTMLElement) {
		document.body.appendChild(node);
		return {
			destroy() {
				node.remove();
			}
		};
	}

	let results: ScoredEntry[] = $derived(searchEntries(index, query, 12));

	$effect(() => {
		void query;
		selected = 0;
	});

	$effect(() => {
		void selected;
		void open;
		if (!open || !list) return;
		const active = list.querySelector(`[data-result="${selected}"]`);
		active?.scrollIntoView({ block: 'nearest' });
	});

	function go(href: string) {
		open = false;
		query = '';
		goto(href);
	}

	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			open = false;
			return;
		}
		if (!open) return;
		if (e.key === 'ArrowDown') {
			e.preventDefault();
			selected = Math.min(selected + 1, results.length - 1);
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			selected = Math.max(selected - 1, 0);
		} else if (e.key === 'Enter') {
			e.preventDefault();
			const r = results[selected];
			if (r) go(r.url);
		}
	}

	function highlight(title: string, q: string): { pre: string; hit: string; post: string } {
		const needle = q.trim();
		if (!needle) return { pre: title, hit: '', post: '' };
		const i = title.toLowerCase().indexOf(needle.toLowerCase());
		if (i < 0) return { pre: title, hit: '', post: '' };
		return { pre: title.slice(0, i), hit: title.slice(i, i + needle.length), post: title.slice(i + needle.length) };
	}

	onMount(() => {
		(async () => {
			try {
				const res = await fetchDataFile(fetch, 'search-index.json');
				if (!res) {
					indexMissing = true;
					return;
				}
				const data = await res.json();
				if (Array.isArray(data)) index = data as SearchEntry[];
				else indexMissing = true;
			} catch {
				indexMissing = true;
			}
		})();
		function global(e: KeyboardEvent) {
			const mod = e.metaKey || e.ctrlKey;
			if ((mod && e.key.toLowerCase() === 'k') || (e.key === '/' && !open)) {
				if (e.key === '/' && isTypingTarget(e.target)) return;
				e.preventDefault();
				open = true;
				queueMicrotask(() => input?.focus());
			}
		}
		// Keystrokes meant for text must never open search: form fields, the
		// shell input, and rich editors (CodeMirror is a contenteditable div,
		// not a textarea).
		function isTypingTarget(t: unknown): boolean {
			if (!(t instanceof HTMLElement)) return false;
			const tag = t.tagName;
			if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
			if (t.isContentEditable) return true;
			return t.closest('[contenteditable="true"], .cm-content, .cm-editor') !== null;
		}
		function external() {
			open = true;
			queueMicrotask(() => input?.focus());
		}
		window.addEventListener('keydown', global);
		window.addEventListener('rnx:open-search', external);
		return () => {
			window.removeEventListener('keydown', global);
			window.removeEventListener('rnx:open-search', external);
		};
	});
</script>

<svelte:window onkeydown={onKey} />

<button
	onclick={() => {
		open = true;
		queueMicrotask(() => input?.focus());
	}}
	class="search-trigger"
	aria-label="Search docs"
>
	<Search size={14} />
	<span>Search</span>
	<kbd class="search-key">⌘K</kbd>
</button>

{#if open}
	<!-- svelte-ignore a11y_click_events_have_key_events: backdrop has an Escape key handler below -->
	<div
		use:portal
		class="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/65 px-4 pb-4 pt-[8vh] backdrop-blur-sm sm:items-center sm:p-8"
		onclick={() => (open = false)}
		onkeydown={(e) => {
			if (e.key === 'Escape') open = false;
		}}
		role="presentation"
	>
		<div
			class="panel modal-pop max-h-[80vh] w-full max-w-2xl overflow-hidden bg-aura-bg"
			role="dialog"
			tabindex="-1"
			aria-modal="true"
			aria-label="Search docs"
			onclick={(e) => e.stopPropagation()}
		>
			<div class="flex items-center gap-3 border-b border-aura-border px-5 py-1">
				<Search size={16} class="shrink-0 text-aura-faint" />
				<input
					bind:this={input}
					bind:value={query}
					placeholder="Search the manual, guide, packages and API"
					class="w-full bg-transparent py-4 text-[15px] text-aura-text outline-none placeholder:text-aura-faint"
					aria-label="Search query"
				/>
				<kbd class="search-key shrink-0">esc</kbd>
			</div>
			<ul bind:this={list} class="max-h-[60vh] overflow-auto p-2">
				{#each results as r, i}
					{@const badge = BADGES[r.category]}
					{@const hl = highlight(r.title, query)}
					<li data-result={i}>
						<button
							onclick={() => go(r.url)}
							onmouseenter={() => (selected = i)}
							class="flex min-h-11 w-full items-center justify-between gap-4 rounded-sm px-3 py-2 text-left text-sm {i ===
							selected
								? 'bg-aura-surfaceElevated text-aura-text'
								: 'text-aura-muted'}"
						>
							<span class="min-w-0">
								{#if r.section}
									<span class="block truncate text-[11px] text-aura-faint">{r.section}</span>
								{/if}
								<span class="block truncate">{hl.pre}{#if hl.hit}<mark
											class="bg-transparent font-semibold text-aura-green">{hl.hit}</mark
										>{/if}{hl.post}</span
								>
							</span>
							<span class="shrink-0 text-[11px] {badge.class}">{badge.label}</span>
						</button>
					</li>
				{/each}
				{#if results.length === 0}
					<li class="px-3 py-4 text-sm text-aura-muted">
						{indexMissing ? 'Search index not available yet.' : 'No matches.'}
					</li>
				{/if}
			</ul>
			<div class="flex items-center gap-4 border-t border-aura-border px-5 py-2.5 text-[11px] text-aura-faint">
				<span><kbd class="search-key">↑</kbd> <kbd class="search-key">↓</kbd> to move</span>
				<span><kbd class="search-key">↵</kbd> to open</span>
			</div>
		</div>
	</div>
{/if}

<style>
	.search-trigger {
		display: none;
		align-items: center;
		gap: var(--space-xs);
		min-height: 36px;
		padding: 0 var(--space-xs) 0 var(--space-sm);
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		font-size: var(--text-xs);
		color: var(--color-muted);
	}

	.search-trigger:hover {
		border-color: var(--color-rule-strong);
		color: var(--color-ink);
	}

	@media (min-width: 1024px) {
		.search-trigger {
			display: inline-flex;
		}
	}

	.search-key {
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-xs);
		padding: 1px 5px;
		font-family: var(--font-mono);
		font-size: 10px;
		line-height: 1.5;
		color: var(--color-faint);
	}
</style>
