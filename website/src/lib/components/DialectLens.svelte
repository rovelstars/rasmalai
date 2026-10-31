<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { getDialect, setDialect, type Dialect } from '$lib/stores/dialect.svelte';
	import LangIcon from '$lib/components/LangIcon.svelte';

	const OPTIONS: { value: Dialect; label: string }[] = [
		{ value: 'none', label: 'None' },
		{ value: 'rust', label: 'Rust' },
		{ value: 'go', label: 'Go' },
		{ value: 'cpp', label: 'C++' },
		{ value: 'typescript', label: 'TypeScript' }
	];

	let { compact = false }: { compact?: boolean } = $props();

	function pick(d: Dialect) {
		setDialect(d);
		const m = page.url.pathname.match(/^\/guide\/rosetta\/from-([^/]+)$/);
		if (m && d !== 'none' && m[1] !== d) {
			goto(`/guide/rosetta/from-${d}`);
		}
	}
</script>

<div
	class="flex items-center gap-px overflow-hidden rounded-sm border border-aura-border"
	role="group"
	aria-label="Dialect lens"
>
	{#if !compact}
		<span class="px-2 font-mono text-[11px] text-aura-faint">lens</span>
	{/if}
	{#each OPTIONS as o}
		<button
			onclick={() => pick(o.value)}
			class="press flex items-center gap-1 px-2 py-1 font-mono text-[11px] {getDialect() === o.value
				? 'bg-aura-purple/10 text-aura-purple'
				: 'text-aura-muted hover:text-aura-text'}"
			aria-pressed={getDialect() === o.value}
		>
			{#if o.value !== 'none'}<LangIcon lang={o.value} size={12} />{/if}
			{o.label}
		</button>
	{/each}
</div>
