<script lang="ts">
	import { ArrowLeft, ArrowRight } from 'lucide-svelte';

	let {
		prev = null,
		next = null
	}: {
		prev?: { path: string; title: string } | null;
		next?: { path: string; title: string } | null;
	} = $props();
</script>

{#if prev || next}
	<nav class="doc-pager" aria-label="Page navigation">
		{#if prev}
			<a href={prev.path} class="doc-pager-link">
				<span class="doc-pager-dir"><ArrowLeft size={12} /> Previous</span>
				<span class="doc-pager-title">{prev.title}</span>
			</a>
		{:else}
			<span></span>
		{/if}
		{#if next}
			<a href={next.path} class="doc-pager-link doc-pager-next">
				<span class="doc-pager-dir">Next <ArrowRight size={12} /></span>
				<span class="doc-pager-title">{next.title}</span>
			</a>
		{/if}
	</nav>
{/if}
