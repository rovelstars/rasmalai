<script lang="ts">
	import BrandMark from '$lib/components/BrandMark.svelte';

	type Link = { href: string; label: string; mark?: 'github' | 'discord' };

	const COLUMNS: { title: string; links: Link[] }[] = [
		{
			title: 'Learn',
			links: [
				{ href: '/guide', label: 'Guide' },
				{ href: '/manual', label: 'Manual' },
				{ href: '/playground', label: 'Playground' }
			]
		},
		{
			title: 'Registry',
			links: [
				{ href: '/packages', label: 'Packages' },
				{ href: '/docs/@std/prelude/overview', label: 'Standard library' },
				{ href: '/etc', label: 'Registry policy' }
			]
		},
		{
			title: 'Project',
			links: [
				{ href: 'https://github.com/rovelstars/rasmalai', label: 'GitHub', mark: 'github' },
				{ href: '/discord', label: 'Discord', mark: 'discord' },
				{ href: '/license', label: 'License' }
			]
		}
	];
</script>

<footer class="site-footer mt-[var(--space-4xl)] border-t border-aura-border">
	<div class="shell">
		<div class="grid gap-10 py-12 md:grid-cols-[minmax(0,1fr)_minmax(0,1.5fr)] md:gap-16 md:py-14">
			<a href="/" class="display self-start text-[2rem] leading-none text-aura-text">
				Rasmalai
			</a>

			<nav class="grid grid-cols-2 gap-x-6 gap-y-8 sm:grid-cols-3" aria-label="Footer">
				{#each COLUMNS as col}
					<div>
						<p class="label">{col.title}</p>
						<ul class="mt-3 space-y-2">
							{#each col.links as l}
								<li>
									<a
										href={l.href}
										class="inline-flex items-center gap-1.5 text-sm text-aura-muted hover:text-aura-text"
									>
										{#if l.mark}<BrandMark name={l.mark} />{/if}
										{l.label}
									</a>
								</li>
							{/each}
						</ul>
					</div>
				{/each}
			</nav>
		</div>

		<div class="flex flex-wrap items-center justify-between gap-3 border-t border-aura-border py-5 text-xs text-aura-faint">
			<p>Rasmalai is dual-licensed under MIT or Apache-2.0. Built by Rovel Stars and contributors.</p>
			<p class="tabular">rnx 0.x</p>
		</div>
	</div>
</footer>
