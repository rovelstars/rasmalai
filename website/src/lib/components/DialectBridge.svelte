<script lang="ts">
	import { marked } from 'marked';
	import { getDialect } from '$lib/stores/dialect.svelte';
	import { Diamond, MoveRight } from 'lucide-svelte';
	import { bridgeFor } from '$lib/docs/bridges';

	let { slug }: { slug: string } = $props();

	let bridge = $derived(bridgeFor(slug, getDialect()));
	let fromHtml = $derived(bridge ? (marked.parseInline(bridge.from) as string) : '');
	let textHtml = $derived(bridge ? (marked.parseInline(bridge.text) as string) : '');
	let label = $derived.by(() => {
		const d = getDialect();
		return d === 'cpp' ? 'C++' : d.charAt(0).toUpperCase() + d.slice(1);
	});
</script>

{#if bridge}
	<aside class="mb-6 mt-4 rounded-r-md border-l-2 border-aura-pink bg-aura-surface/60 p-4">
		<p class="flex items-center gap-1.5 font-mono text-xs uppercase tracking-wider text-aura-pink"><Diamond size={11} />Bridge from {label}</p>
		<p class="mt-1 text-sm leading-relaxed">
			<span class="bridge-md font-mono text-[13px] text-aura-muted">{@html fromHtml}</span>
			<span class="bridge-md text-aura-text"><MoveRight size={12} class="mr-1 inline text-aura-muted" />{@html textHtml}</span>
		</p>
	</aside>
{/if}

<style>
	.bridge-md :global(code) {
		border-radius: 0.25rem;
		background-color: rgba(255, 255, 255, 0.06);
		padding: 0.1rem 0.375rem;
		font-size: 13px;
	}
	:global(html.light) .bridge-md :global(code) {
		background-color: rgba(0, 0, 0, 0.05);
	}
</style>
