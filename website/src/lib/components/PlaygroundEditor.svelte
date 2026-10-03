<script lang="ts">
	import { highlightAura } from '$lib/aura/highlight';

	let {
		value = $bindable(''),
		onRun
	}: {
		value: string;
		onRun: () => void;
	} = $props();

	let area: HTMLTextAreaElement | null = $state(null);
	let back: HTMLPreElement | null = $state(null);
	let gutter: HTMLDivElement | null = $state(null);

	let spans = $derived(highlightAura(value + '\n'));
	let lineCount = $derived(value.split('\n').length);

	function syncScroll() {
		if (area && back) {
			back.scrollTop = area.scrollTop;
			back.scrollLeft = area.scrollLeft;
		}
		if (area && gutter) {
			gutter.scrollTop = area.scrollTop;
		}
	}

	function onKey(e: KeyboardEvent) {
		if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
			e.preventDefault();
			onRun();
			return;
		}
		if (e.key === 'Tab' && area) {
			e.preventDefault();
			const start = area.selectionStart ?? 0;
			const end = area.selectionEnd ?? 0;
			value = value.slice(0, start) + '    ' + value.slice(end);
			queueMicrotask(() => {
				if (!area) return;
				area.selectionStart = area.selectionEnd = start + 4;
			});
		}
	}
</script>

<div class="flex min-h-[320px] font-mono text-[16px] leading-relaxed md:text-[13px]">
	<div
		bind:this={gutter}
		class="w-11 shrink-0 select-none overflow-hidden border-r border-aura-border py-4 text-right text-aura-muted"
		aria-hidden="true"
	>
		{#each Array(lineCount) as _, i}
			<div class="pr-2">{i + 1}</div>
		{/each}
	</div>
	<div class="relative min-w-0 flex-1">
		<pre
			bind:this={back}
			class="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre p-4"
			aria-hidden="true"><code>{#each spans as s}<span
						class={s.cls}>{s.text}</span
					>{/each}</code></pre>
		<textarea
			bind:this={area}
			bind:value
			oninput={syncScroll}
			onscroll={syncScroll}
			onkeydown={onKey}
			spellcheck={false}
			autocomplete="off"
			autocapitalize="off"
			wrap="off"
			class="absolute inset-0 resize-none whitespace-pre bg-transparent p-4 text-transparent caret-aura-purple outline-none selection:bg-aura-purple/30"
			aria-label="Rasmalai source editor"
		></textarea>
	</div>
</div>
