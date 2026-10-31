<script lang="ts">
	import type { Component } from 'svelte';
	import { ArrowLeft, ArrowRight } from 'lucide-svelte';
	import { onDocCodeClick } from '$lib/docs/codeActions';
	import { attachSectionCopy, onSectionCopyClick } from '$lib/docs/sectionCopy';

	let {
		title,
		html,
		raw,
		sections,
		prev = null,
		next = null,
		icon: Icon = null,
		tone = 'signal'
	}: {
		title: string;
		html: string;
		raw?: string;
		sections?: { id: string; markdown: string }[];
		prev?: { path: string; title: string } | null;
		next?: { path: string; title: string } | null;
		icon?: Component<{ size?: number; strokeWidth?: number; class?: string }> | null;
		tone?: 'signal' | 'accent';
	} = $props();

	let articleEl: HTMLElement | undefined = $state();

	$effect(() => {
		if (articleEl) attachSectionCopy(articleEl, raw, sections);
	});
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events: delegated code-block handler -->
<article
	class="doc-prose"
	bind:this={articleEl}
	onclick={(e) => {
		onDocCodeClick(e);
		onSectionCopyClick(e);
	}}
>
	<div class="doc-toolbar">
		<button type="button" class="doc-btn" data-copy-page title="Copy the full page as markdown">
			copy page
		</button>
	</div>

	<h1 class="doc-h1">
		{#if Icon}
			<Icon size={22} strokeWidth={2} class="doc-h1-icon {tone === 'signal' ? 'is-signal' : 'is-accent'}" />
		{/if}
		{title}
	</h1>

	<!-- eslint-disable-next-line svelte/no-at-html-tags -->
	{@html html}

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
</article>
