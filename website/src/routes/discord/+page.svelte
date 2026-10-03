<script lang="ts">
	import BrandMark from '$lib/components/BrandMark.svelte';
	import { Diamond } from 'lucide-svelte';
</script>

<svelte:head>
	<title>Community Discord — Rasmalai</title>
</svelte:head>

<main class="mx-auto max-w-3xl px-4 pb-16 pt-8">
	<p class="flex items-center gap-1.5 font-mono text-sm tracking-wide text-aura-purple"><Diamond size={12} />community</p>
	<h1 class="mt-3 text-3xl font-bold tracking-tight">Discord</h1>
	<div class="panel mt-6 p-6">
		<p class="leading-relaxed text-aura-muted">
			Ask questions, show what you are building, and shape the language with the
			people writing it. All skill levels welcome.
		</p>
		<a
			href="https://discord.com/invite/eWbt297SkU"
			class="press mt-4 inline-flex items-center gap-2 rounded-md bg-aura-purple px-5 py-2.5 text-sm font-semibold text-[#15141b]"
		>
			<BrandMark name="discord" size={16} />
			Join the server
		</a>
	</div>
</main>
