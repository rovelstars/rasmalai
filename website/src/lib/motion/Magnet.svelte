<script lang="ts">
	import type { Snippet } from 'svelte';

	let { children, strength = 7 }: { children: Snippet; strength?: number } = $props();

	let el: HTMLElement | undefined = $state();
	let reduced =
		typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;

	function onMove(e: MouseEvent) {
		if (reduced || !el) return;
		const r = el.getBoundingClientRect();
		const dx = e.clientX - (r.left + r.width / 2);
		const dy = e.clientY - (r.top + r.height / 2);
		el.style.transform = `translate(${(dx / r.width) * strength}px, ${(dy / r.height) * strength}px)`;
	}

	function onLeave() {
		if (el) el.style.transform = '';
	}
</script>

<span
	bind:this={el}
	onmousemove={onMove}
	onmouseleave={onLeave}
	role="presentation"
	class="magnet"
	>{@render children()}</span
>

<style>
	.magnet {
		display: inline-block;
		transition: transform 0.25s cubic-bezier(0.16, 1, 0.3, 1);
		will-change: transform;
	}
	@media (prefers-reduced-motion: reduce) {
		.magnet {
			transition: none;
		}
	}
</style>
