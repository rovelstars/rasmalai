<script lang="ts">
	import { ArrowRight, Diamond } from 'lucide-svelte';
	import { ETC_SECTIONS } from '$lib/docs/nav';
	import { reveal } from '$lib/motion/reveal';
</script>

<svelte:head>
	<title>Registry & Policy — Rasmalai</title>
	<meta
		name="description"
		content="Non-language rules of the Rasmalai registry: publishing, versioning, abuse handling, and tiers."
	/>
</svelte:head>

<p class="flex items-center gap-1.5 font-mono text-sm tracking-wide text-aura-purple"><Diamond size={12} />registry & policy</p>
<h1 class="mt-3 text-3xl font-bold tracking-tight">Rules of the Registry</h1>
<p class="mt-2 max-w-[65ch] text-aura-muted">
	Everything non-language: what may be published, how versions and yank work,
	what counts as abuse, and what the tiers cover. Normative where stated.
</p>
<div class="mt-6 grid gap-3">
	{#each ETC_SECTIONS as s}
		<a use:reveal href={s.path} class="press panel group block p-4 hover:border-aura-borderHover">
			<p class="inline-flex items-center gap-2 font-semibold">
				{#if s.icon}<s.icon size={16} strokeWidth={1.75} class="text-aura-purple" />{/if}
				{s.title}
				<ArrowRight size={14} class="transition-transform group-hover:translate-x-0.5" />
			</p>
			<p class="mt-1 text-sm text-aura-muted">{s.description}</p>
		</a>
	{/each}
</div>
