<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { GitBranch, FlaskConical, BookOpen, BookMarked, Package, Search, Diamond, Star } from 'lucide-svelte';
	import DocSearch from '$lib/components/DocSearch.svelte';

	type Theme = 'auto' | 'dark' | 'light';
	let theme = $state<Theme>('dark');
	let stars = $state('2.4k');
	let signInOpen = $state(false);

	function apply(t: Theme) {
		theme = t;
		try {
			localStorage.setItem('rnx-theme', t);
		} catch {
			/* private mode */
		}
		const dark =
			t === 'dark' || (t === 'auto' && window.matchMedia('(prefers-color-scheme: dark)').matches);
		document.documentElement.classList.toggle('light', !dark);
		document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
	}

	onMount(() => {
		let saved: Theme = 'dark';
		try {
			const v = localStorage.getItem('rnx-theme');
			if (v === 'auto' || v === 'dark' || v === 'light') saved = v;
		} catch {
			/* ignore */
		}
		apply(saved);
		fetch('https://api.github.com/repos/rovelstars/rasmalai')
			.then((r) => (r.ok ? r.json() : null))
			.then((d) => {
				if (d && typeof d.stargazers_count === 'number') {
					stars =
						d.stargazers_count >= 1000
							? (d.stargazers_count / 1000).toFixed(1) + 'k'
							: String(d.stargazers_count);
				}
			})
			.catch(() => {});
		function onKey(e: KeyboardEvent) {
			if (e.key === 'Escape') signInOpen = false;
		}
		window.addEventListener('keydown', onKey);
		return () => window.removeEventListener('keydown', onKey);
	});

	const modes: Theme[] = ['auto', 'dark', 'light'];
</script>

<header
	class="sticky top-0 z-40 border-b border-aura-border bg-aura-bg/90 backdrop-blur"
>
	<nav class="mx-auto flex h-16 max-w-7xl items-center gap-4 px-4">
		<a href="/" class="flex shrink-0 items-center gap-2" aria-label="Rasmalai home">
			<img src="/favicon.svg" alt="" class="h-9 w-9" />
			<span class="font-semibold tracking-tight">Rasmalai</span>
		</a>
		<div class="hidden flex-1 items-center justify-center gap-4 text-sm lg:flex lg:gap-7">
			<a
				href="/guide"
				aria-current={page.url.pathname.startsWith('/guide') ? 'page' : undefined}
				class={page.url.pathname.startsWith('/guide') ? 'text-aura-text' : 'text-aura-muted hover:text-aura-text'}><span class="inline-flex min-h-8 items-center gap-1.5"><BookOpen size={14} />Guide</span></a
			>
			<a
				href="/manual"
				aria-current={page.url.pathname.startsWith('/manual') ? 'page' : undefined}
				class={page.url.pathname.startsWith('/manual') ? 'text-aura-text' : 'text-aura-muted hover:text-aura-text'}><span class="inline-flex min-h-8 items-center gap-1.5"><BookMarked size={14} />Manual</span></a
			>
			<a
				href="/packages"
				aria-current={page.url.pathname.startsWith('/packages') ? 'page' : undefined}
				class={page.url.pathname.startsWith('/packages') ? 'text-aura-text' : 'text-aura-muted hover:text-aura-text'}><span class="inline-flex min-h-8 items-center gap-1.5"><Package size={14} />Packages</span></a
			>
			<a
				href="/playground"
				aria-current={page.url.pathname.startsWith('/playground') ? 'page' : undefined}
				class={page.url.pathname.startsWith('/playground') ? 'text-aura-text' : 'text-aura-muted hover:text-aura-text'}><span class="inline-flex min-h-8 items-center gap-1.5"><FlaskConical size={14} />Playground</span></a
			>
		</div>
		<div class="ml-auto flex items-center gap-2 lg:ml-0">
			<DocSearch />
			<div
				class="flex items-center rounded-full border border-aura-border p-0.5 font-mono text-[11px]"
				role="group"
				aria-label="Theme"
			>
				{#each modes as m}
					<button
						onclick={() => apply(m)}
						class="press rounded-full px-2 py-0.5 capitalize {theme === m
							? 'bg-aura-surfaceElevated text-aura-text'
							: 'text-aura-muted hover:text-aura-text'}"
						aria-pressed={theme === m}
					>
						{m}
					</button>
				{/each}
			</div>
			<a
				href="https://github.com/rovelstars/rasmalai"
				class="press hidden items-center gap-1.5 rounded-full border border-aura-border px-3 py-1 text-sm text-aura-muted hover:border-aura-borderHover hover:text-aura-text lg:flex"
			>
				<GitBranch size={14} strokeWidth={1.5} />
				<span class="inline-flex items-center gap-1 tabular font-mono text-xs"><Star size={11} />{stars}</span>
			</a>
			<button
				onclick={() => (signInOpen = true)}
				class="press rounded-full border border-aura-border px-3 py-1 text-sm hover:border-aura-borderHover"
			>
				Sign in
			</button>
		</div>
	</nav>
	<div class="flex items-center gap-1 overflow-x-auto border-t border-aura-border px-2 py-1 text-sm lg:hidden">
		<button
			onclick={() => window.dispatchEvent(new CustomEvent('rnx:open-search'))}
			class="inline-flex min-h-8 shrink-0 items-center gap-1.5 px-2 text-aura-muted"
			aria-label="Search docs"
		>
			<Search size={16} />Search
		</button>
		<a href="/guide" class="inline-flex min-h-8 shrink-0 items-center gap-1.5 px-2 text-aura-muted"><BookOpen size={14} />Guide</a>
		<a href="/manual" class="inline-flex min-h-8 shrink-0 items-center gap-1.5 px-2 text-aura-muted"><BookMarked size={14} />Manual</a>
		<a href="/packages" class="inline-flex min-h-8 shrink-0 items-center gap-1.5 px-2 text-aura-muted"><Package size={14} />Packages</a>
		<a href="/playground" class="inline-flex min-h-8 shrink-0 items-center gap-1.5 px-2 text-aura-muted"><FlaskConical size={14} />Playground</a>
	</div>
</header>

{#if signInOpen}
	<!-- svelte-ignore a11y_click_events_have_key_events: backdrop has an Escape key handler below -->
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
		onclick={() => (signInOpen = false)}
		onkeydown={(e) => {
			if (e.key === 'Escape') signInOpen = false;
		}}
		role="presentation"
	>
		<div
			class="panel w-full max-w-md p-6"
			role="dialog"
			tabindex="-1"
			aria-modal="true"
			aria-label="Sign in preview"
			onclick={(e) => e.stopPropagation()}
		>
			<p class="flex items-center gap-1.5 font-mono text-xs tracking-wide text-aura-purple"><Diamond size={11} />developer registry</p>
			<h2 class="mt-2 text-xl font-semibold">Reserve your namespace</h2>
			<p class="mt-2 text-sm leading-relaxed text-aura-muted">
				Developer registry accounts are launching in v1.1. Connect GitHub now to reserve
				your namespace handle.
			</p>
			<div class="mt-5 flex gap-2">
				<button
					class="press flex flex-1 items-center justify-center gap-2 rounded-md bg-aura-purple px-4 py-2 text-sm font-medium text-[#15141b]"
				>
					<GitBranch size={15} strokeWidth={1.5} /> Connect GitHub
				</button>
				<button
					onclick={() => (signInOpen = false)}
					class="press rounded-md border border-aura-border px-4 py-2 text-sm text-aura-muted hover:text-aura-text"
				>
					Close
				</button>
			</div>
		</div>
	</div>
{/if}
