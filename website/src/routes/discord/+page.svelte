<script lang="ts">
	import BrandMark from '$lib/components/BrandMark.svelte';
</script>

<svelte:head>
	<title>Community Discord | Rasmalai</title>
	<meta
		name="description"
		content="Ask questions, share what you are building, and shape Rasmalai with the people writing it."
	/>
</svelte:head>

<main class="shell pb-24 pt-14">
	<header>
		<p class="font-mono text-xs text-aura-faint">community</p>
		<h1 class="display-s mt-4 max-w-[16ch]">Discord</h1>
		<p class="lede mt-5">
			Ask questions, show what you are building, and shape the language with the people writing
			it. All skill levels welcome, including your first day with a systems language.
		</p>
		<a href="https://discord.com/invite/eWbt297SkU" class="btn btn-primary mt-8">
			<BrandMark name="discord" size={16} />
			Join the server
		</a>
	</header>

	<section class="index-section" aria-labelledby="channels">
		<div class="index-section-head">
			<span class="index-section-num">01</span>
			<h2 id="channels" class="index-section-title">Where to start</h2>
			<p class="index-section-note">Three channels cover most of the traffic.</p>
		</div>
		<ul class="chan-list">
			<li class="chan">
				<span class="chan-num">01</span>
				<span class="chan-name">#help</span>
				<span class="chan-desc">
					Compiler errors, install trouble, and semantics questions. Paste the diagnostic code
					and the snippet that produced it.
				</span>
			</li>
			<li class="chan">
				<span class="chan-num">02</span>
				<span class="chan-name">#showcase</span>
				<span class="chan-desc">
					Programs, benchmarks, and packages you shipped. A screenshot of a running binary
					beats a paragraph about it.
				</span>
			</li>
			<li class="chan">
				<span class="chan-num">03</span>
				<span class="chan-name">#compiler-dev</span>
				<span class="chan-desc">
					LIR, codegen, and the memory model. Read the manual section before proposing a
					semantic change.
				</span>
			</li>
		</ul>
	</section>

	<p class="index-foot">
		Prefer written reports?
		<a href="/etc" class="inline-flex items-center gap-1 text-aura-signal underline underline-offset-4">
			Read the registry policy
		</a>
		before filing a package dispute.
	</p>
</main>

<style>
	.chan-list {
		margin-top: var(--space-md);
		border-top: 1px solid var(--color-rule);
	}

	.chan {
		display: grid;
		grid-template-columns: 44px minmax(0, 1fr) minmax(0, 1.15fr);
		gap: var(--space-md);
		align-items: baseline;
		border-bottom: 1px solid var(--color-rule);
		padding: var(--space-sm) 0;
	}

	.chan-num {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.chan-name {
		font-family: var(--font-mono);
		font-size: var(--text-sm);
		color: var(--color-ink);
	}

	.chan-desc {
		font-size: var(--text-sm);
		line-height: 1.55;
		color: var(--color-muted);
	}

	@media (max-width: 720px) {
		.chan {
			grid-template-columns: 32px minmax(0, 1fr);
			row-gap: var(--space-2xs);
		}

		.chan-desc {
			grid-column: 2;
		}
	}
</style>
