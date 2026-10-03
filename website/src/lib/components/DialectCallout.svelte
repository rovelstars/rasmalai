<script lang="ts">
	import { Diamond, MoveRight } from 'lucide-svelte';
	import { isDialectActive, type Dialect } from '$lib/stores/dialect.svelte';

	let {
		target,
		from,
		children
	}: {
		target: Exclude<Dialect, 'none'>;
		from: string;
		children: import('svelte').Snippet;
	} = $props();

	const label = $derived(
		target === 'cpp' ? 'C++' : target.charAt(0).toUpperCase() + target.slice(1)
	);
</script>

{#if isDialectActive(target)}
	<aside class="my-4 rounded-r-md border-l-2 border-aura-pink bg-aura-surface/60 p-4">
		<p class="flex items-center gap-1.5 font-mono text-xs uppercase tracking-wider text-aura-pink"><Diamond size={11} />Bridge from {label}</p>
		<p class="mt-1 text-sm leading-relaxed">
			<span class="font-mono text-[13px] text-aura-muted">{from}</span> <MoveRight size={12} class="inline shrink-0 text-aura-muted" />
			<span class="text-aura-text">{@render children()}</span>
		</p>
	</aside>
{/if}
