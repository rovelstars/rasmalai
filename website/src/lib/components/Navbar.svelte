<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { Menu, X, Star, Search, Sun, Moon, Monitor, ArrowUpRight } from 'lucide-svelte';
	import DocSearch from '$lib/components/DocSearch.svelte';

	type Theme = 'auto' | 'dark' | 'light';
	let theme = $state<Theme>('dark');
	let stars = $state('2.4k');
	let signInOpen = $state(false);
	let menuOpen = $state(false);

	const LINKS = [
		{ href: '/guide', label: 'Guide' },
		{ href: '/manual', label: 'Manual' },
		{ href: '/packages', label: 'Packages' },
		{ href: '/playground', label: 'Playground' }
	];

	const MODES: { id: Theme; label: string; icon: typeof Sun }[] = [
		{ id: 'auto', label: 'Match system theme', icon: Monitor },
		{ id: 'dark', label: 'Dark theme', icon: Moon },
		{ id: 'light', label: 'Light theme', icon: Sun }
	];

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

	function isActive(href: string): boolean {
		return page.url.pathname === href || page.url.pathname.startsWith(`${href}/`);
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
			if (e.key === 'Escape') {
				signInOpen = false;
				menuOpen = false;
			}
		}
		window.addEventListener('keydown', onKey);
		return () => window.removeEventListener('keydown', onKey);
	});
</script>

<header class="site-header">
	<nav class="shell flex h-[68px] items-center gap-6" aria-label="Main">
		<a href="/" class="wordmark" aria-label="Rasmalai home">
			<img src="/favicon.svg" alt="" class="h-[22px] w-[22px]" />
			<span class="text-[16px] font-semibold tracking-tight">Rasmalai</span>
		</a>

		<div class="hidden flex-1 items-center gap-6 md:flex md:pl-4">
			{#each LINKS as l}
				<a
					href={l.href}
					aria-current={isActive(l.href) ? 'page' : undefined}
					class="nav-link"
					class:nav-link-active={isActive(l.href)}
				>
					{l.label}
				</a>
			{/each}
		</div>

		<div class="ml-auto flex items-center gap-2 md:ml-0">
			<DocSearch />

			<button
				onclick={() => window.dispatchEvent(new CustomEvent('rnx:open-search'))}
				class="icon-btn inline-flex md:hidden"
				aria-label="Search docs"
			>
				<Search size={17} />
			</button>

			<div class="theme-switch hidden sm:flex" role="group" aria-label="Theme">
				{#each MODES as m}
					<button
						onclick={() => apply(m.id)}
						class="theme-option"
						class:theme-option-active={theme === m.id}
						aria-pressed={theme === m.id}
						aria-label={m.label}
						title={m.label}
					>
						<m.icon size={13} />
					</button>
				{/each}
			</div>

			<a
				href="https://github.com/rovelstars/rasmalai"
				class="gh-link hidden lg:flex"
				aria-label="Rasmalai on GitHub"
			>
				<Star size={12} />
				<span class="tabular text-[12px]">{stars}</span>
			</a>

			<button onclick={() => (signInOpen = true)} class="btn btn-ghost hidden h-9 min-h-0 px-3.5 text-[13px] sm:inline-flex">
				Sign in
			</button>

			<button
				onclick={() => (menuOpen = !menuOpen)}
				class="icon-btn inline-flex md:hidden"
				aria-label={menuOpen ? 'Close menu' : 'Open menu'}
				aria-expanded={menuOpen}
			>
				{#if menuOpen}<X size={18} />{:else}<Menu size={18} />{/if}
			</button>
		</div>
	</nav>

	{#if menuOpen}
		<div class="mobile-sheet md:hidden">
			<div class="shell flex flex-col gap-1 pb-5">
				{#each LINKS as l}
					<a
						href={l.href}
						aria-current={isActive(l.href) ? 'page' : undefined}
						class="mobile-link"
						onclick={() => (menuOpen = false)}
					>
						{l.label}
					</a>
				{/each}
				<div class="mt-4 flex items-center justify-between border-t border-aura-border pt-4">
					<span class="label">Theme</span>
					<div class="theme-switch flex" role="group" aria-label="Theme">
						{#each MODES as m}
							<button
								onclick={() => apply(m.id)}
								class="theme-option"
								class:theme-option-active={theme === m.id}
								aria-pressed={theme === m.id}
								aria-label={m.label}
							>
								<m.icon size={14} />
							</button>
						{/each}
					</div>
				</div>
				<a
					href="https://github.com/rovelstars/rasmalai"
					class="mt-2 flex items-center justify-between py-2 text-sm text-aura-muted"
				>
					GitHub <span class="inline-flex items-center gap-1.5 tabular text-[13px]"><Star size={12} />{stars}</span>
				</a>
				<button
					onclick={() => {
						signInOpen = true;
						menuOpen = false;
					}}
					class="mt-2 flex items-center justify-between py-2 text-left text-sm text-aura-muted"
				>
					Sign in <ArrowUpRight size={14} />
				</button>
			</div>
		</div>
	{/if}
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
			class="panel w-full max-w-md bg-aura-bg p-7"
			role="dialog"
			tabindex="-1"
			aria-modal="true"
			aria-label="Sign in preview"
			onclick={(e) => e.stopPropagation()}
		>
			<h2 class="display text-[2rem] leading-[1.05]">Reserve your namespace</h2>
			<p class="mt-3 text-sm leading-relaxed text-aura-muted">
				Developer registry accounts are launching in v1.1. Connect GitHub now to reserve
				your namespace handle.
			</p>
			<div class="mt-6 flex gap-2">
				<button class="btn btn-primary flex-1">
					Connect GitHub
				</button>
				<button
					onclick={() => (signInOpen = false)}
					class="btn btn-ghost"
				>
					Close
				</button>
			</div>
		</div>
	</div>
{/if}

<style>
	.site-header {
		position: sticky;
		top: 0;
		z-index: var(--z-sticky-nav);
		border-bottom: 1px solid var(--color-rule);
		background: var(--color-paper);
	}

	.wordmark {
		display: inline-flex;
		flex-shrink: 0;
		align-items: center;
		gap: var(--space-xs);
		color: var(--color-ink);
	}

	.nav-link {
		position: relative;
		display: inline-flex;
		min-height: 32px;
		align-items: center;
		font-size: var(--text-sm);
		color: var(--color-muted);
		transition: color var(--dur-micro) ease;
	}

	.nav-link:hover {
		color: var(--color-ink);
	}

	.nav-link-active {
		color: var(--color-ink);
	}

	.nav-link-active::after {
		content: '';
		position: absolute;
		left: 0;
		right: 0;
		bottom: 1px;
		height: 1px;
		background: var(--color-accent);
	}

	.icon-btn {
		height: 36px;
		width: 36px;
		align-items: center;
		justify-content: center;
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		color: var(--color-muted);
	}

	.icon-btn:hover {
		border-color: var(--color-rule-strong);
		color: var(--color-ink);
	}

	.theme-switch {
		align-items: center;
		gap: 2px;
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		padding: 2px;
	}

	.theme-option {
		display: inline-flex;
		height: 30px;
		width: 28px;
		align-items: center;
		justify-content: center;
		border-radius: 2px;
		color: var(--color-faint);
	}

	.theme-option:hover {
		color: var(--color-ink);
	}

	.theme-option-active {
		background: rgb(var(--aura-text) / 0.08);
		color: var(--color-ink);
	}

	.gh-link {
		align-items: center;
		gap: 5px;
		padding: 0 var(--space-xs);
		color: var(--color-muted);
	}

	.gh-link:hover {
		color: var(--color-ink);
	}

	.mobile-sheet {
		border-top: 1px solid var(--color-rule);
		background: var(--color-paper);
		animation: fade-in 0.18s ease-out;
	}

	.mobile-link {
		display: flex;
		min-height: 44px;
		align-items: center;
		border-bottom: 1px solid var(--color-rule);
		font-family: var(--font-display);
		font-size: 1.35rem;
		color: var(--color-ink);
	}

	.mobile-link[aria-current='page'] {
		color: var(--color-accent);
	}
</style>
