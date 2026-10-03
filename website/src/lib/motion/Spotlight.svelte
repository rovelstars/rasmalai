<script lang="ts">
	import type { Snippet } from 'svelte';

	let { children, class: cls = '' }: { children: Snippet; class?: string } = $props();

	function onMove(e: MouseEvent) {
		const t = e.currentTarget as HTMLElement;
		const r = t.getBoundingClientRect();
		t.style.setProperty('--mx', `${e.clientX - r.left}px`);
		t.style.setProperty('--my', `${e.clientY - r.top}px`);
	}
</script>

<div onmousemove={onMove} role="presentation" class="spot {cls}">
	{@render children()}
</div>

<style>
	.spot {
		position: relative;
	}
	.spot::before {
		content: '';
		position: absolute;
		inset: 0;
		border-radius: inherit;
		pointer-events: none;
		opacity: 0;
		transition: opacity 0.35s ease;
		background: radial-gradient(
			260px circle at var(--mx, 50%) var(--my, 50%),
			rgba(162, 119, 255, 0.14),
			transparent 65%
		);
	}
	.spot:hover::before {
		opacity: 1;
	}
</style>
