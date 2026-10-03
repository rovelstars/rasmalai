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
	class="flex items-center gap-0.5 rounded-full border border-aura-border p-0.5"
	role="group"
	aria-label="Dialect lens"
>
	{#if !compact}
		<span class="px-2 font-mono text-[11px] text-aura-muted">lens</span>
	{/if}
	{#each OPTIONS as o}
		<button
			onclick={() => pick(o.value)}
			class="press flex items-center gap-1 rounded-full px-2 py-0.5 font-mono text-[11px] font-medium {getDialect() === o.value
				? 'border border-aura-purple/40 bg-aura-surfaceElevated text-aura-purple'
				: 'border border-transparent text-aura-muted hover:text-aura-text'}"
			aria-pressed={getDialect() === o.value}
		>
			{#if o.value !== 'none'}<LangIcon lang={o.value} size={12} />{/if}
			{o.label}
		</button>
	{/each}
</div>
