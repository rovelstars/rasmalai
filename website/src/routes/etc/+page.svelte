<script lang="ts">
	import { ArrowRight } from 'lucide-svelte';
	import { ETC_SECTIONS } from '$lib/docs/nav';
</script>

<svelte:head>
	<title>Registry &amp; Policy | Rasmalai</title>
	<meta
		name="description"
		content="Non-language rules of the Rasmalai registry: publishing, versioning, abuse handling, and tiers."
	/>
</svelte:head>

<div class="index-page">
	<header class="index-head">
		<h1 class="index-title">Registry rules and operations</h1>
		<p class="index-lede">
			Everything that is not the language: what may be published, how versions and yank work, what
			counts as abuse, and what each capability tier promises. Normative where stated.
		</p>
	</header>

	<section class="index-section" aria-labelledby="policies">
		<div class="index-section-head">
			<h2 id="policies" class="index-section-title">Policies</h2>
			<p class="index-section-note">Read in order on a first pass; jump to a clause when something breaks.</p>
		</div>
		<ol class="index-list">
			{#each ETC_SECTIONS as s}
				<li>
					<a href={s.path} class="index-row">
						<span class="index-row-title">
							<span class="grid h-4 w-4 shrink-0 place-items-center text-aura-faint">
								<s.icon size={14} strokeWidth={1.7} />
							</span>
							{s.title}
						</span>
						<span class="index-row-desc">{s.description}</span>
					</a>
				</li>
			{/each}
		</ol>
	</section>

	<p class="index-foot">
		Publishing a package?
		<a href="/packages" class="inline-flex items-center gap-1 text-aura-signal underline underline-offset-4">
			Browse the registry <ArrowRight size={12} />
		</a>
	</p>
</div>
