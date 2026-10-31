<script lang="ts">
	import { ArrowRight } from 'lucide-svelte';
	import { MANUAL_SECTIONS, MANUAL_PARTS } from '$lib/docs/nav';
</script>

<svelte:head>
	<title>Language Manual | Rasmalai</title>
	<meta
		name="description"
		content="The normative Rasmalai reference: numerics, memory, protocols, concurrency, and toolchain."
	/>
</svelte:head>

<div class="index-page">
	<header class="index-head">
		<h1 class="index-title">The language manual</h1>
		<p class="index-lede">
			The normative reference: execution semantics, compiler lowering, memory guarantees, and
			performance characteristics. For the friendly path, start with
			<a href="/guide" class="text-aura-signal underline underline-offset-4">the guide</a>.
		</p>
	</header>

	{#each MANUAL_PARTS as part, pi}
		<section class="index-section" aria-labelledby="part-{pi}">
			<div class="index-section-head">
				<h2 id="part-{pi}" class="index-section-title">{part}</h2>
			</div>
			<ul class="index-list">
				{#each MANUAL_SECTIONS.filter((m) => m.part === part) as m}
					<li>
						<a href={m.path} class="index-row">
							<span class="index-row-title">
								<span class="grid h-4 w-4 shrink-0 place-items-center text-aura-faint">
									<m.icon size={14} strokeWidth={1.7} />
								</span>
								{m.title}
							</span>
							<span class="index-row-desc">{m.description}</span>
						</a>
					</li>
				{/each}
			</ul>
		</section>
	{/each}

	<p class="index-foot">
		Looking for per-module APIs?
		<a href="/docs/@std/prelude/overview" class="inline-flex items-center gap-1 text-aura-link underline underline-offset-4">
			Browse the standard library <ArrowRight size={12} />
		</a>
	</p>
</div>
