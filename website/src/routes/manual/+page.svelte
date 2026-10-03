<script lang="ts">
	import { ArrowRight, Diamond } from 'lucide-svelte';
	import { MANUAL_SECTIONS } from '$lib/docs/nav';
	import { reveal } from '$lib/motion/reveal';
</script>

<svelte:head>
	<title>Language Manual — Rasmalai</title>
	<meta
		name="description"
		content="The normative Rasmalai reference: numerics, memory, protocols, concurrency, and toolchain."
	/>
</svelte:head>

<p class="flex items-center gap-1.5 font-mono text-sm tracking-wide text-aura-purple"><Diamond size={12} />language manual</p>
<h1 class="mt-3 text-3xl font-bold tracking-tight">Exact Semantics, Stated Plainly</h1>
<p class="mt-2 max-w-[65ch] text-aura-muted">
	The Manual is the normative reference: execution semantics, compiler lowering,
	memory guarantees, and performance characteristics. For the friendly learning
	path, start with <a href="/guide" class="text-aura-green hover:underline">the Guide</a>
	instead.
</p>

<div class="mt-6 grid gap-3 md:grid-cols-2">
	{#each MANUAL_SECTIONS as m, i}
		<a href={m.path} use:reveal={{ delay: Math.min(i, 6) * 45 }} class="panel lift block p-4 hover:border-aura-borderHover">
			<p class="flex items-center gap-2 font-semibold">
				<span class="font-mono text-xs text-aura-muted">{String(i + 1).padStart(2, '0')}</span>
				<m.icon size={15} strokeWidth={1.75} class="shrink-0 text-aura-purple" />
				{m.title}
			</p>
			<p class="mt-1 pl-6 text-sm text-aura-muted">{m.description}</p>
		</a>
	{/each}
</div>

<p class="mt-10 border-t border-aura-border pt-5 text-aura-muted">
	Looking for per-module APIs?
	<a href="/docs/@std/simd/overview" class="text-aura-purple hover:underline"
		><span class="inline-flex items-center gap-1">Explore the Standard Library Reference <ArrowRight size={13} /></span></a
	>
</p>
