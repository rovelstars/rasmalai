<script lang="ts">
	import { ExternalLink } from 'lucide-svelte';
	import AuraCode from '$lib/components/AuraCode.svelte';
	import type { DocModule, JsDoc } from '$lib/docs/api';
	import { renderMarkdown } from '$lib/docs/markdown';
import { buildSymbolIndex, linkifyProseHtml, linkifyTypeText } from '$lib/docs/symbollinks';
import { stdApiModules } from '$lib/docs/stdlib';
	import { encodeSnippet } from '$lib/playground/share';

	let { mod, idPrefix = '', typeLinks, allMods = null, symbolHref = null }: {
		mod: DocModule;
		idPrefix?: string;
		typeLinks?: Map<string, string>;
		allMods?: DocModule[] | null;
		symbolHref?: ((moduleName: string, anchor: string) => string) | null;
	} = $props();

	let symbolIndex = $derived(
		buildSymbolIndex(allMods ?? stdApiModules() ?? [mod], symbolHref ?? undefined)
	);

	let resolvedTypeLinks = $derived(typeLinks ?? symbolIndex);

	function md(text: string, exclude?: string): string {
		try {
			const html = renderMarkdown(text).html;
			return linkifyProseHtml(html, symbolIndex, exclude ? new Set([exclude]) : undefined);
		} catch {
			return text;
		}
	}

	function playgroundUrl(code: string): string {
		return `/playground?code=${encodeSnippet(code)}`;
	}

	function paramsOf(docs: JsDoc) {
		return docs.tags.filter((t) => t.kind === 'param');
	}
	function returnsOf(docs: JsDoc) {
		return docs.tags.filter((t) => t.kind === 'returns' || t.kind === 'return');
	}
	function throwsOf(docs: JsDoc) {
		return docs.tags.filter((t) => t.kind === 'throws' || t.kind === 'error');
	}
	function examplesOf(docs: JsDoc) {
		return docs.tags.filter((t) => t.kind === 'example');
	}
</script>

{#each mod.classes as c}
	<section id="{idPrefix}class-{c.name}" class="panel mt-6 scroll-mt-24 overflow-hidden">
		<div class="border-b border-aura-border px-4 py-2">
			<span class="font-mono text-xs text-aura-pink">class{' '}</span><span
				class="font-mono text-sm font-semibold">{c.name}</span
			>
		</div>
		<div class="p-4">
			{#if c.docs.description}<div class="text-sm [&>p]:m-0">{@html md(c.docs.description, c.name)}</div>{/if}
			{#if c.init}
				<p class="mt-2 font-mono text-[13px] text-aura-muted">{c.init}</p>
			{/if}
			{#if c.fields.length > 0}
				<h3 class="mt-4 font-mono text-xs uppercase tracking-wider text-aura-muted">fields</h3>
				<ul class="mt-1 space-y-1 font-mono text-[13px]">
					{#each c.fields as f}
						<li><span class="text-aura-text">{f.name}</span><span class="text-aura-pink">: <!-- eslint-disable-next-line svelte/no-at-html-tags -->{@html linkifyTypeText(f.ty, symbolIndex)}</span></li>
					{/each}
				</ul>
			{/if}
			{#each c.methods as m}
				<div class="mt-4 border-t border-aura-border pt-3">
					<AuraCode source={m.sig} links={resolvedTypeLinks} />
					{#if m.docs.description}<div class="mt-2 text-sm [&>p]:m-0">{@html md(m.docs.description, m.name)}</div>{/if}
					{#each paramsOf(m.docs) as p}
						<p class="mt-1 font-mono text-[13px]">
							<span class="text-aura-cyan">{p.name}</span>
							<span class="text-aura-muted"> — {p.text}</span>
						</p>
					{/each}
					{#each returnsOf(m.docs) as r}
						<p class="mt-1 text-sm"><span class="font-mono text-[13px] text-aura-green">returns</span> — {r.text}</p>
					{/each}
					{#each throwsOf(m.docs) as t}
						<p class="mt-1 rounded border border-aura-red/40 px-2 py-1 text-sm">
							<span class="font-mono text-[13px] text-aura-red">throws</span> — {t.text}
						</p>
					{/each}
					{#each examplesOf(m.docs) as e}
						<div class="mt-2">
							<AuraCode source={e.text} showCopy />
							<a
								href={playgroundUrl(e.text)}
								rel="external"
								class="press mt-2 inline-flex items-center gap-1.5 rounded border border-aura-border px-2.5 py-1 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
							>
								<ExternalLink size={12} />Run in Playground
							</a>
						</div>
					{/each}
				</div>
			{/each}
		</div>
	</section>
{/each}

{#each mod.enums as e}
	<section id="{idPrefix}enum-{e.name}" class="panel mt-6 scroll-mt-24 overflow-hidden">
		<div class="border-b border-aura-border px-4 py-2">
			<span class="font-mono text-xs text-aura-pink">enum{' '}</span><span
				class="font-mono text-sm font-semibold">{e.name}</span
			>
		</div>
		<ul class="space-y-1 p-4 font-mono text-[13px]">
			{#each e.variants as v}
				<li>
					<span class="text-aura-text">{v.name}</span>{#if v.payload.length > 0}<span
							class="text-aura-pink">({v.payload.join(', ')})</span
						>{/if}
				</li>
			{/each}
		</ul>
	</section>
{/each}

{#if mod.functions.length > 0}
	<h2 class="mt-8 text-xl font-bold">Functions</h2>
	{#each mod.functions as f}
		<section id="{idPrefix}fn-{f.name}" class="panel mt-4 scroll-mt-24 overflow-hidden">
			<div class="p-4">
				<AuraCode source={f.sig} links={resolvedTypeLinks} />
				{#if f.docs.description}<div class="mt-2 text-sm [&>p]:m-0">{@html md(f.docs.description, f.name)}</div>{/if}
				{#each paramsOf(f.docs) as p}
					<p class="mt-1 font-mono text-[13px]">
						<span class="text-aura-cyan">{p.name}</span>
						<span class="text-aura-muted"> — {p.text}</span>
					</p>
				{/each}
				{#each returnsOf(f.docs) as r}
					<p class="mt-1 text-sm"><span class="font-mono text-[13px] text-aura-green">returns</span> — {r.text}</p>
				{/each}
				{#each examplesOf(f.docs) as e}
					<div class="mt-2">
						<AuraCode source={e.text} showCopy />
						<a
							href={playgroundUrl(e.text)}
							rel="external"
							class="press mt-2 inline-flex items-center gap-1.5 rounded border border-aura-border px-2.5 py-1 font-mono text-xs text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
						>
							<ExternalLink size={12} />Run in Playground
						</a>
					</div>
				{/each}
			</div>
		</section>
	{/each}
{/if}

{#if mod.constants.length > 0}
	<h2 class="mt-8 text-xl font-bold">Constants</h2>
	<ul class="mt-2 space-y-1 font-mono text-[13px]">
		{#each mod.constants as c}
			<li id="{idPrefix}const-{c.name}" class="scroll-mt-24">
				<span class="text-aura-text">{c.name}</span><span class="text-aura-pink">: {c.ty}</span>
			</li>
		{/each}
	</ul>
{/if}

<style>
	:global(a.symlink) {
		color: inherit;
		text-decoration: none;
	}
	:global(a.symlink:hover) {
		text-decoration: underline;
	}
</style>
