<script lang="ts">
	import { reveal } from '$lib/motion/reveal';

	let {
		text,
		class: cls = '',
		delay = 0,
		stagger = 22
	}: { text: string; class?: string; delay?: number; stagger?: number } = $props();

	let words = $derived(text.split(' '));
	let n = $derived.by(() => {
		let i = 0;
		return words.map((w) => {
			const start = i;
			i += [...w].length;
			return { w, start };
		});
	});
</script>

<span use:reveal={{ delay }} class="st {cls}" style="--st-stagger:{stagger}ms" aria-label={text}>
	{#each n as { w, start }, wi}
		<span class="st-word" aria-hidden="true"
			>{#each [...w] as ch, ci}<span class="st-char" style="--i:{start + ci}">{ch}</span
				>{/each}{#if wi < n.length - 1}{' '}{/if}</span
		>
	{/each}
</span>

<style>
	.st-word {
		display: inline-block;
		white-space: pre;
	}
	.st-char {
		display: inline-block;
		opacity: 0;
		transform: translateY(0.45em);
		filter: blur(4px);
		transition:
			opacity 0.55s cubic-bezier(0.16, 1, 0.3, 1),
			transform 0.55s cubic-bezier(0.16, 1, 0.3, 1),
			filter 0.55s cubic-bezier(0.16, 1, 0.3, 1);
		transition-delay: calc(var(--rv-delay, 0ms) + var(--i) * var(--st-stagger, 22ms));
		will-change: opacity, transform;
	}
	.st.is-in .st-char {
		opacity: 1;
		transform: none;
		filter: none;
	}
	@media (prefers-reduced-motion: reduce) {
		.st-char {
			opacity: 1;
			transform: none;
			filter: none;
			transition: none;
		}
	}
</style>
