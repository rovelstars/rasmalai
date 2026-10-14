<script lang="ts">
	import { onMount } from 'svelte';
	import HeroWorkbench from '$lib/components/HeroWorkbench.svelte';
	import ParetoScatter from '$lib/components/ParetoScatter.svelte';
	import TradeoffSwitchboard from '$lib/components/TradeoffSwitchboard.svelte';
	import ToolchainStation from '$lib/components/ToolchainStation.svelte';
	import Journey from '$lib/journey/Journey.svelte';
	import BackgroundFX from '$lib/components/BackgroundFX.svelte';
	import SquareField from '$lib/components/SquareField.svelte';
	import { Diamond } from 'lucide-svelte';
	import SplitText from '$lib/motion/SplitText.svelte';
	import Scramble from '$lib/motion/Scramble.svelte';
	import { reveal } from '$lib/motion/reveal';
	import { CANONICAL_HOST } from '$lib/site';

	let copied = $state(false);
	let host = $state(CANONICAL_HOST);
	let protocol = $state('https:');
	type OsKind = 'unix' | 'windows';
	let os = $state<OsKind>('unix');
	let installCmd = $derived(
		os === 'windows' ? `irm ${protocol}//${host}/install.ps1 | iex` : `curl -fsSL ${protocol}//${host}/install.sh | sh`
	);

	let segEl: HTMLDivElement | undefined = $state();
	let pill = $state({ left: 0, width: 0, ready: false });

	function measurePill() {
		const root = segEl;
		if (!root) return;
		const order: OsKind[] = ['unix', 'windows'];
		const btns = root.querySelectorAll('button');
		const at = btns[order.indexOf(os)] as HTMLElement | undefined;
		if (!at) return;
		pill = { left: at.offsetLeft, width: at.offsetWidth, ready: true };
	}

	function selectOs(next: OsKind) {
		os = next;
		requestAnimationFrame(() => measurePill());
	}

	onMount(() => {
		host = window.location.host;
		protocol = window.location.protocol;
		const ua = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData;
		const hint = ua?.platform ?? navigator.platform ?? navigator.userAgent ?? '';
		if (/win/i.test(hint)) os = 'windows';
		else os = 'unix';
		measurePill();
		const onResize = () => measurePill();
		window.addEventListener('resize', onResize);
		return () => window.removeEventListener('resize', onResize);
	});

	async function copyInstall() {
		const done = (ok: boolean) => {
			copied = ok;
			if (ok) setTimeout(() => (copied = false), 1500);
		};
		try {
			if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
				await navigator.clipboard.writeText(installCmd);
				done(true);
				return;
			}
		} catch {
			// fall through to the legacy path below
		}
		done(legacyCopy(installCmd));
	}

	function legacyCopy(text: string): boolean {
		try {
			const ta = document.createElement('textarea');
			ta.value = text;
			ta.setAttribute('readonly', '');
			ta.style.position = 'fixed';
			ta.style.opacity = '0';
			document.body.appendChild(ta);
			ta.select();
			ta.setSelectionRange(0, ta.value.length);
			const ok = document.execCommand('copy');
			document.body.removeChild(ta);
			return ok;
		} catch {
			return false;
		}
	}
</script>

<div class="chassis">
	<BackgroundFX />
	<div class="relative z-10 mx-auto max-w-7xl px-4">
		<!-- hero intro -->
		<div class="relative">
			<SquareField />
			<section class="relative mx-auto max-w-3xl pt-14 pb-10 text-center lg:pt-20">
			<p use:reveal class="flex items-center justify-center gap-2 font-mono text-sm tracking-wide text-aura-purple">
				<Diamond size={12} class="shrink-0" /><Scramble text="sweet to write, fast to run" />
			</p>
			<h1 use:reveal={{ delay: 100 }} class="mt-4 text-5xl font-bold leading-[1.02] tracking-[-0.03em] md:text-7xl">
				<SplitText text="Memory safety without the negotiations." />
			</h1>
			<p use:reveal={{ delay: 250 }} class="mx-auto mt-5 max-w-[58ch] leading-relaxed text-aura-muted">
				Scopes own every value, so memory frees on time, every time. No garbage collector,
				no borrow checker, no cleanup to forget. One rnx binary checks, runs, tests,
				and ships it. The workbench below is live — type and watch the compiler check it.
			</p>
			<div use:reveal={{ delay: 350 }} class="mt-7 flex flex-col items-center gap-3">
				<a
					href="/playground"
					class="btn-star press inline-block rounded-md bg-aura-purple px-5 py-2.5 text-sm font-bold text-aura-bg"
					>Try it in your browser</a
				>
				<button
					onclick={copyInstall}
					class="press max-w-full rounded-full border backdrop-blur transition-colors duration-200 {copied ? 'border-aura-green/60 bg-aura-surface/70' : 'border-aura-border bg-aura-surface/70 hover:border-aura-borderHover'} px-5 py-2.5 font-mono text-sm text-aura-text"
				>
					<span class="inline-flex min-w-0 items-center overflow-hidden align-bottom">
						{#key os}
							<span class="cmd-roll inline-block whitespace-nowrap"><span class="text-aura-muted">{os === 'windows' ? '>' : '$'}</span> <span class="break-all">{installCmd}</span></span>
						{/key}
					</span>
					<span class="ml-1 text-xs text-aura-muted">{copied ? '- copied' : '- copy'}</span>
				</button>
				<div bind:this={segEl} class="relative flex items-center gap-1 rounded-full border border-aura-border bg-aura-surface/70 px-1 py-1 backdrop-blur" role="group" aria-label="Operating system">
					<span
						aria-hidden="true"
						class="os-pill pointer-events-none absolute top-1 bottom-1 rounded-full bg-aura-purple"
						style:left="{pill.left}px"
						style:width="{pill.width}px"
						style:opacity={pill.ready ? 1 : 0}
					></span>
					{#each [['unix', 'Linux / macOS'], ['windows', 'Windows']] as [id, label]}
						<button
							onclick={() => selectOs(id as typeof os)}
							aria-pressed={os === id}
							class="press relative z-10 rounded-full px-2.5 py-1 font-mono text-xs transition-colors duration-200 {os === id ? 'text-aura-bg' : 'text-aura-muted hover:text-aura-text'}"
						>
							{label}
						</button>
					{/each}
				</div>
			</div>
		</section>
		</div>

		<!-- bay 01 -->
		<section aria-label="Execution workbench">
			<div use:reveal class="flex items-center gap-3 border-t border-aura-border pt-5 pb-4">
				<span class="font-mono text-sm text-aura-purple" aria-hidden="true">+</span>
				<h2 class="font-mono text-xs tracking-[0.2em] text-aura-muted">01 - LIVE WORKBENCH</h2>
				<span class="h-px flex-1 bg-aura-surfaceElevated" aria-hidden="true"></span>
				<span class="hidden font-mono text-[11px] text-aura-muted sm:inline">[ BAY: 0x01 ]</span>
			</div>
			<div use:reveal>
				<HeroWorkbench />
			</div>
		</section>

		<!-- bay 02 -->
		<section aria-label="How it works" class="mt-12">
			<div use:reveal class="flex items-center gap-3 border-t border-aura-border pt-5 pb-4">
				<span class="font-mono text-sm text-aura-purple" aria-hidden="true">+</span>
				<h2 class="font-mono text-xs tracking-[0.2em] text-aura-muted">02 - HOW IT FLOWS</h2>
				<span class="h-px flex-1 bg-aura-surfaceElevated" aria-hidden="true"></span>
				<span class="hidden font-mono text-[11px] text-aura-muted sm:inline">[ BAY: 0x02 ]</span>
			</div>
			<p use:reveal class="max-w-[65ch] pb-4 leading-relaxed text-aura-muted">
				Follow one file from keystroke to binary — through every tool, pass, and backend.
			</p>
			<div use:reveal>
				<Journey />
			</div>
		</section>

		<!-- bay 03 -->
		<section aria-label="Benchmarks" class="mt-12">
			<div use:reveal class="flex items-center gap-3 border-t border-aura-border pt-5 pb-4">
				<span class="font-mono text-sm text-aura-purple" aria-hidden="true">+</span>
				<h2 class="font-mono text-xs tracking-[0.2em] text-aura-muted">03 - BROWSER PROVING GROUND</h2>
				<span class="h-px flex-1 bg-aura-surfaceElevated" aria-hidden="true"></span>
				<span class="hidden font-mono text-[11px] text-aura-muted sm:inline">[ BAY: 0x03 ]</span>
			</div>
			<p use:reveal class="max-w-[65ch] pb-4 leading-relaxed text-aura-muted">
				Seven workloads, seven toolchains, one log-log plot. Bottom-left wins: fastest
				execution with the smallest footprint. Turn the dial and check the methodology below.
			</p>
			<div use:reveal>
				<ParetoScatter />
			</div>
		</section>

		<!-- bay 04 -->
		<section aria-label="Tradeoffs" class="mt-12">
			<div use:reveal class="flex items-center gap-3 border-t border-aura-border pt-5 pb-4">
				<span class="font-mono text-sm text-aura-purple" aria-hidden="true">+</span>
				<h2 class="font-mono text-xs tracking-[0.2em] text-aura-muted">04 - TRADEOFF SWITCHBOARD</h2>
				<span class="h-px flex-1 bg-aura-surfaceElevated" aria-hidden="true"></span>
				<span class="hidden font-mono text-[11px] text-aura-muted sm:inline">[ BAY: 0x04 ]</span>
			</div>
			<p use:reveal class="max-w-[65ch] pb-4 leading-relaxed text-aura-muted">
				Every tool charges a tax. Flip through the ones you have paid and see what
				Rasmalai does instead.
			</p>
			<div use:reveal>
				<TradeoffSwitchboard />
			</div>
		</section>

		<!-- bay 05 -->
		<section aria-label="Toolchain" class="mt-12">
			<div use:reveal class="flex items-center gap-3 border-t border-aura-border pt-5 pb-4">
				<span class="font-mono text-sm text-aura-purple" aria-hidden="true">+</span>
				<h2 class="font-mono text-xs tracking-[0.2em] text-aura-muted">05 - TOOLCHAIN STATION</h2>
				<span class="h-px flex-1 bg-aura-surfaceElevated" aria-hidden="true"></span>
				<span class="hidden font-mono text-[11px] text-aura-muted sm:inline">[ BAY: 0x05 ]</span>
			</div>
			<p use:reveal class="max-w-[65ch] pb-4 leading-relaxed text-aura-muted">
				One binary does it all. Switch modes and poke at it — the lint fixer and the
				test runner respond.
			</p>
			<div use:reveal>
				<ToolchainStation />
			</div>
		</section>
	</div>
</div>

<style>
	.chassis ::selection {
		background: rgba(162, 119, 255, 0.35);
	}
	.chassis :global(.bay) {
		border: 1px solid rgba(255, 255, 255, 0.1);
		background-color: #14131a;
	}
	:global(html.light) .chassis :global(.bay) {
		border-color: rgba(0, 0, 0, 0.12);
		background-color: #ffffff;
	}
</style>
