<script lang="ts">
	import { setDialect } from '$lib/stores/dialect.svelte';
	import { GUIDE_CHAPTERS, MANUAL_SECTIONS, ROSETTA, TRACKS } from '$lib/docs/nav';
	import { ArrowRight, Diamond } from 'lucide-svelte';
	import LangIcon from '$lib/components/LangIcon.svelte';
	import { reveal } from '$lib/motion/reveal';

	function pick(dialect: (typeof ROSETTA)[number]['dialect'], path: string) {
		setDialect(dialect);
		window.location.href = path;
	}
</script>

<svelte:head>
	<title>The Guide — Learn Rasmalai</title>
	<meta
		name="description"
		content="The friendly, progressive path into Rasmalai: rosetta guides, and tracks."
	/>
</svelte:head>

<p class="flex items-center gap-1.5 font-mono text-sm tracking-wide text-aura-green"><Diamond size={12} />the guide</p>
<h1 class="mt-3 text-3xl font-bold tracking-tight">Learn Rasmalai, One Chapter at a Time</h1>
<p class="mt-2 max-w-[65ch] text-aura-muted">
	A friendly, progressive path from zero to productive — every example runnable
	in the browser. For exact semantics, see the
	<a href="/manual" class="text-aura-purple hover:underline">Language Manual</a>.
</p>

<h2 class="mt-10 text-xl font-bold">Chapters</h2>
<div class="mt-3 grid gap-3 md:grid-cols-2">
	{#each GUIDE_CHAPTERS as c, i}
		<a href={c.path} use:reveal={{ delay: Math.min(i, 7) * 45 }} class="panel lift block p-4 hover:border-aura-borderHover">
			<p class="flex items-center gap-2 font-semibold">
				<span class="font-mono text-xs text-aura-muted">{String(i + 1).padStart(2, '0')}</span>
				<c.icon size={15} strokeWidth={1.75} class="shrink-0 text-aura-green" />
				{c.title}
			</p>
			<p class="mt-1 pl-6 text-sm text-aura-muted">{c.description}</p>
		</a>
	{/each}
</div>

<h2 id="rosetta" class="mt-10 scroll-mt-24 text-xl font-bold">Coming from another language?</h2>
<p class="mt-1 max-w-[65ch] text-sm text-aura-muted">
	Rosetta guides translate what you already know — every example shown side-by-side
	with Rasmalai.
</p>
<div class="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
	{#each ROSETTA as r, i}
		<button
			onclick={() => pick(r.dialect, r.path)}
			use:reveal={{ delay: Math.min(i, 3) * 60 }}
			class="panel lift p-4 text-left hover:border-aura-borderHover"
		>
			<p class="flex items-center gap-2 font-mono text-sm font-bold text-aura-purple">
				<span class="grid h-4 w-4 shrink-0 place-items-center">
					<LangIcon lang={r.dialect} size={16} cls="h-full w-full" />
				</span>
				{r.label}
			</p>
			<p class="mt-1 text-sm text-aura-muted">{r.blurb}</p>
		</button>
	{/each}
</div>

<h2 class="mt-10 text-xl font-bold">Guided tracks</h2>
<div class="mt-3 grid gap-4 md:grid-cols-2">
	{#each TRACKS as t}
		<a href="/guide/tracks/{t.slug}" class="panel lift block p-5 hover:border-aura-borderHover">
			<p class="flex items-center gap-2 font-mono text-xs text-aura-green">
				<t.icon size={14} strokeWidth={1.75} />
				{t.slug === 'fast-track' ? '15 minutes - systems engineers' : 'start here - newcomers'}
			</p>
			<h3 class="mt-1 text-lg font-bold">{t.title}</h3>
			<p class="mt-1 text-sm text-aura-muted">{t.description}</p>
		</a>
	{/each}
</div>

<h2 class="mt-10 text-xl font-bold">Go deeper</h2>
<div class="mt-3 grid gap-3 md:grid-cols-2">
	{#each MANUAL_SECTIONS.slice(0, 4) as m}
		<a href={m.path} class="panel press block p-4 hover:border-aura-borderHover">
			<p class="flex items-center gap-2 font-semibold">
				<m.icon size={15} strokeWidth={1.75} class="shrink-0 text-aura-purple" />
				{m.title}
			</p>
			<p class="mt-1 pl-6 text-sm text-aura-muted">{m.description}</p>
		</a>
	{/each}
	<a href="/manual" class="panel lift block p-4 hover:border-aura-borderHover">
		<p class="flex items-center gap-2 font-semibold">
			<span class="text-aura-purple" aria-hidden="true">#</span>
			Full Language Manual <ArrowRight size={13} class="inline" />
		</p>
		<p class="mt-1 pl-6 text-sm text-aura-muted">All seven normative sections.</p>
	</a>
</div>

<p class="mt-10 border-t border-aura-border pt-5 text-aura-muted">
	Already know what you're looking for?
	<a href="/docs/@std/simd/overview" class="text-aura-green hover:underline"
		><span class="inline-flex items-center gap-1">Explore the Standard Library Reference <ArrowRight size={13} /></span></a
	>
</p>
