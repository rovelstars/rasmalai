<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { searchEntries, type SearchEntry, type ScoredEntry } from '$lib/docs/search';

	// search-index.json is generated at build time (prebuild) into
	// static/data and served same-origin, so no build carries a snapshot.
	// Point a deploy somewhere else by editing this export.
	export const searchIndexUrl = '/data/search-index.json';

	interface Badge {
		label: string;
		class: string;
	}

	const BADGES: Record<SearchEntry['category'], Badge> = {
		manual: { label: 'Manual', class: 'border-emerald-500/40 bg-emerald-500/10 text-emerald-400' },
		guide: { label: 'Guide', class: 'border-sky-500/40 bg-sky-500/10 text-sky-400' },
		package: { label: 'Package', class: 'border-purple-500/40 bg-purple-500/10 text-purple-400' },
		symbol: { label: 'API', class: 'border-amber-500/40 bg-amber-500/10 text-amber-400' }
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
				const res = await fetch(searchIndexUrl);
				if (!res.ok) {
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
				const tag = (e.target as HTMLElement)?.tagName;
				if (e.key === '/' && (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT')) return;
				e.preventDefault();
				open = true;
				queueMicrotask(() => input?.focus());
			}
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
	class="press hidden items-center gap-2 rounded-md border border-aura-border px-2.5 py-1 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text lg:flex"
	aria-label="Search docs"
>
	<span>search docs</span>
	<span class="rounded border border-aura-border px-1">⌘K</span>
</button>

{#if open}
	<!-- svelte-ignore a11y_click_events_have_key_events: backdrop has an Escape key handler below -->
	<div
		use:portal
		class="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/60 px-4 pb-4 pt-[8vh] sm:items-center sm:p-8"
		onclick={() => (open = false)}
		onkeydown={(e) => {
			if (e.key === 'Escape') open = false;
		}}
		role="presentation"
	>
		<div
			class="panel modal-pop max-h-[80vh] w-full max-w-2xl overflow-hidden"
			role="dialog"
			tabindex="-1"
			aria-modal="true"
			aria-label="Search docs"
			onclick={(e) => e.stopPropagation()}
		>
			<input
				bind:this={input}
				bind:value={query}
				placeholder="concepts, chapters, @std symbols..."
				class="w-full border-b border-aura-border bg-transparent px-4 py-3 text-sm outline-none placeholder:text-aura-muted"
				aria-label="Search query"
			/>
			<ul bind:this={list} class="max-h-72 overflow-auto p-1.5">
				{#each results as r, i}
					{@const badge = BADGES[r.category]}
					{@const hl = highlight(r.title, query)}
					<li data-result={i}>
						<button
							onclick={() => go(r.url)}
							onmouseenter={() => (selected = i)}
							class="flex min-h-8 w-full items-center justify-between gap-3 rounded px-2.5 py-1.5 text-left text-sm {i ===
							selected
								? 'bg-aura-surfaceElevated text-aura-text'
								: 'text-aura-muted'}"
						>
							<span class="min-w-0">
								{#if r.section}
									<span class="block truncate font-mono text-[10px] uppercase tracking-wider opacity-60">{r.section}</span>
								{/if}
								<span class="block truncate font-mono text-[13px]">{hl.pre}{#if hl.hit}<mark
											class="bg-transparent font-semibold text-aura-green">{hl.hit}</mark
										>{/if}{hl.post}</span
								>
							</span>
							<span class="shrink-0 rounded border px-1.5 py-0.5 font-mono text-[10px] {badge.class}">{badge.label}</span>
						</button>
					</li>
				{/each}
				{#if results.length === 0}
					<li class="px-2.5 py-3 text-sm text-aura-muted">
						{indexMissing ? 'Search index not available yet.' : 'No matches.'}
					</li>
				{/if}
			</ul>
		</div>
	</div>
{/if}
