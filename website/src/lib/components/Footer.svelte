<script lang="ts">
	import { BookOpen, FlaskConical, Package, Scale } from 'lucide-svelte';
	import BrandMark from '$lib/components/BrandMark.svelte';
	import { reveal } from '$lib/motion/reveal';
</script>

<footer class="relative z-10 mx-auto max-w-7xl px-4">
	<div class="mt-12 border-t border-aura-border pt-8 pb-6">
		<nav use:reveal class="grid grid-cols-2 gap-x-2 gap-y-6 text-sm sm:grid-cols-3" aria-label="Footer">
			<div>
				<p class="font-mono text-[11px] tracking-[0.2em] text-aura-muted">LEARN</p>
				<div class="mt-2 flex flex-col items-start gap-2">
					<a href="/guide" class="flex items-center gap-2 text-aura-muted hover:text-aura-text"
						><BookOpen size={14} />Guide</a
					>
					<a href="/playground" class="flex items-center gap-2 text-aura-muted hover:text-aura-text"
						><FlaskConical size={14} />Playground</a
					>
					<a href="/packages" class="flex items-center gap-2 text-aura-muted hover:text-aura-text"
						><Package size={14} />Packages</a
					>
				</div>
			</div>
			<div>
				<p class="font-mono text-[11px] tracking-[0.2em] text-aura-muted">COMMUNITY</p>
				<div class="mt-2 flex flex-col items-start gap-2">
					<a
						href="https://github.com/rovelstars/rasmalai"
						class="flex items-center gap-2 text-aura-muted hover:text-aura-text"
						><BrandMark name="github" />GitHub</a
					>
					<a href="/discord" class="flex items-center gap-2 text-aura-muted hover:text-aura-text"
						><BrandMark name="discord" />Discord</a
					>
				</div>
			</div>
			<div>
				<p class="font-mono text-[11px] tracking-[0.2em] text-aura-muted">LEGAL</p>
				<div class="mt-2 flex flex-col items-start gap-2">
					<a href="/license" class="flex items-center gap-2 text-aura-muted hover:text-aura-text"
						><Scale size={14} />License (MIT)</a
					>
				</div>
			</div>
		</nav>
	</div>
</footer>
