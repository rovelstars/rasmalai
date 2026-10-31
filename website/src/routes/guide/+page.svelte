<script lang="ts">
	import { setDialect } from '$lib/stores/dialect.svelte';
	import { GUIDE_CHAPTERS, ROSETTA, TRACKS } from '$lib/docs/nav';
	import { ArrowRight } from 'lucide-svelte';
	import LangIcon from '$lib/components/LangIcon.svelte';

	function pick(dialect: (typeof ROSETTA)[number]['dialect'], path: string) {
		setDialect(dialect);
		window.location.href = path;
	}
</script>

<svelte:head>
	<title>The Guide | Learn Rasmalai</title>
	<meta
		name="description"
		content="The friendly, progressive path into Rasmalai: rosetta guides, and tracks."
	/>
</svelte:head>

<div class="index-page">
	<header class="index-head">
		<h1 class="index-title">Learn Rasmalai</h1>
		<p class="index-lede">
			A progressive path from an empty folder to working programs. Every example runs in the
			browser. For exact semantics, read the <a href="/manual" class="text-aura-link underline underline-offset-4">language manual</a>.
		</p>
	</header>

	<section class="index-section" aria-labelledby="chapters">
		<div class="index-section-head">
			<h2 id="chapters" class="index-section-title">Chapters</h2>
			<p class="index-section-note">{GUIDE_CHAPTERS.length} chapters, in reading order.</p>
		</div>
		<ol class="index-list">
			{#each GUIDE_CHAPTERS as c}
				<li>
					<a href={c.path} class="index-row">
						<span class="index-row-title">
							<span class="grid h-4 w-4 shrink-0 place-items-center text-aura-faint">
								<c.icon size={14} strokeWidth={1.7} />
							</span>
							{c.title}
						</span>
						<span class="index-row-desc">{c.description}</span>
					</a>
				</li>
			{/each}
		</ol>
	</section>

	<section class="index-section" aria-labelledby="rosetta">
		<div class="index-section-head">
			<h2 id="rosetta" class="index-section-title">Coming from another language</h2>
			<p class="index-section-note">Side-by-side translations of what you already know.</p>
		</div>
		<ul class="index-list">
			{#each ROSETTA as r}
				<li>
					<button
						onclick={() => pick(r.dialect, r.path)}
						class="index-row w-full text-left"
					>
						<span class="index-row-title">
							<span class="grid h-4 w-4 shrink-0 place-items-center">
								<LangIcon lang={r.dialect} size={15} cls="h-full w-full" />
							</span>
							{r.title}
						</span>
						<span class="index-row-desc">{r.blurb}</span>
					</button>
				</li>
			{/each}
		</ul>
	</section>

	<section class="index-section" aria-labelledby="tracks">
		<div class="index-section-head">
			<h2 id="tracks" class="index-section-title">Guided tracks</h2>
			<p class="index-section-note">Pick a pace: fifteen minutes, or zero to systems.</p>
		</div>
		<ul class="index-list">
			{#each TRACKS as t}
				<li>
					<a href="/guide/tracks/{t.slug}" class="index-row">
						<span class="index-row-title">
							<span class="grid h-4 w-4 shrink-0 place-items-center text-aura-faint">
								<t.icon size={14} strokeWidth={1.7} />
							</span>
							{t.title}
						</span>
						<span class="index-row-desc">{t.description}</span>
					</a>
				</li>
			{/each}
		</ul>
	</section>

	<p class="index-foot">
		Know what you are looking for?
		<a href="/docs/@std/prelude/overview" class="inline-flex items-center gap-1 text-aura-signal underline underline-offset-4">
			Browse the standard library <ArrowRight size={12} />
		</a>
	</p>
</div>
