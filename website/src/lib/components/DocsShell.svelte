<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { TocEntry } from '$lib/docs/markdown';
	import { Menu, X } from 'lucide-svelte';
	import { page } from '$app/state';

	let {
		label,
		nav,
		children,
		toc = [],
		header,
		drawerLabel = label
	}: {
		label: string;
		nav: Snippet;
		children: Snippet;
		toc?: TocEntry[];
		header?: Snippet;
		drawerLabel?: string;
	} = $props();

	let activeId = $state('');
	let drawer = $state(false);
	let scroller: HTMLElement | null = $state(null);
	let shellActive = $state(false);

	$effect(() => {
		const mq = window.matchMedia('(min-width: 1024px)');
		const sync = () => (shellActive = mq.matches);
		sync();
		mq.addEventListener('change', sync);
		return () => mq.removeEventListener('change', sync);
	});

	$effect(() => {
		void page.url.pathname;
		void shellActive;
		activeId = '';
		drawer = false;
		scroller?.scrollTo({ top: 0 });
		const root = document.querySelector('[data-docs-content]');
		if (!root) return;
		const targets = root.querySelectorAll('[id]');
		if (targets.length === 0) return;
		// In the locked desktop shell the article is its own scroller, so the
		// spy must observe against it; on smaller screens the window scrolls.
		const obs = new IntersectionObserver(
			(entries) => {
				for (const e of entries) {
					if (e.isIntersecting) activeId = (e.target as HTMLElement).id;
				}
			},
			{ root: shellActive && scroller ? scroller : undefined, rootMargin: '-20% 0px -70% 0px' }
		);
		targets.forEach((t) => obs.observe(t));
		return () => obs.disconnect();
	});

	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape') drawer = false;
	}
</script>

<svelte:window onkeydown={onKey} />

<div class="docs app-shell-docs">
	<div class="shell">
		<div class="docs-bar">
			<button
				onclick={() => (drawer = true)}
				class="docs-menu lg:hidden"
				aria-label="Open {drawerLabel.toLowerCase()} index"
			>
				<Menu size={15} />
				<span>Contents</span>
			</button>
			<div class="min-w-0 flex-1">
				{#if header}{@render header()}{/if}
			</div>
		</div>

		<div class="docs-grid" class:docs-grid-wide={toc.length === 0}>
			<nav class="docs-nav hidden lg:block" aria-label={label}>
				<div class="docs-nav-inner">
					{@render nav()}
				</div>
			</nav>

			<main class="min-w-0" data-docs-content bind:this={scroller}>
				{@render children()}
			</main>

			{#if toc.length > 0}
				<aside class="docs-toc hidden xl:block" aria-label="On this page">
					<div class="docs-toc-inner">
						<p class="docs-group">On this page</p>
						<ul class="docs-toc-list">
							{#each toc as t}
								<li class:docs-toc-sub={t.level === 3}>
									<a href="#{t.id}" aria-current={activeId === t.id ? 'location' : undefined}>
										{t.text}
									</a>
								</li>
							{/each}
						</ul>
					</div>
				</aside>
			{/if}
		</div>

		<footer class="docs-shellfoot" aria-label="Site">
			<span>Rasmalai is dual-licensed under MIT or Apache-2.0.</span>
			<span class="docs-shellfoot-links">
				<a href="https://github.com/rovelstars/rasmalai">GitHub</a>
				<a href="/discord">Discord</a>
				<a href="/license">License</a>
			</span>
		</footer>
	</div>
</div>

{#if drawer}
	<!-- svelte-ignore a11y_click_events_have_key_events: the window Escape handler closes the drawer -->
	<div class="fixed inset-0 z-50 lg:hidden" onclick={() => (drawer = false)} role="presentation">
		<div class="absolute inset-0 bg-black/60" aria-hidden="true"></div>
		<nav
			class="drawer-slide absolute bottom-0 left-0 top-0 w-[min(84vw,320px)] overflow-auto border-r border-aura-border bg-aura-bg px-5 py-4"
			aria-label={drawerLabel}
		>
			<div class="mb-4 flex items-center justify-between border-b border-aura-border pb-3">
				<span class="docs-group !mt-0">{drawerLabel}</span>
				<button onclick={() => (drawer = false)} class="docs-close" aria-label="Close index">
					<X size={15} />
				</button>
			</div>
			{@render nav()}
		</nav>
	</div>
{/if}
