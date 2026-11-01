<script lang="ts">
	import { onMount } from 'svelte';
	import HeroWorkbench from '$lib/components/HeroWorkbench.svelte';
	import ParetoScatter from '$lib/components/ParetoScatter.svelte';
	import TradeoffSwitchboard from '$lib/components/TradeoffSwitchboard.svelte';
	import ToolchainStation from '$lib/components/ToolchainStation.svelte';
	import Journey from '$lib/journey/Journey.svelte';
	import ScopeLedger from '$lib/components/ScopeLedger.svelte';
	import RegistryLedger from '$lib/components/RegistryLedger.svelte';
	import HeapField from '$lib/hero/HeapField.svelte';
	import BackgroundFX from '$lib/components/BackgroundFX.svelte';
	import { CANONICAL_HOST } from '$lib/site';

	let copied = $state(false);
	let host = $state(CANONICAL_HOST);
	let protocol = $state('https:');
	type OsKind = 'unix' | 'windows';
	let os = $state<OsKind>('unix');
	let installCmd = $derived(
		os === 'windows' ? `irm ${protocol}//${host}/install.ps1 | iex` : `curl -fsSL ${protocol}//${host}/install.sh | sh`
	);

	onMount(() => {
		host = window.location.host;
		protocol = window.location.protocol;
		const ua = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData;
		const hint = ua?.platform ?? navigator.platform ?? navigator.userAgent ?? '';
		os = /win/i.test(hint) ? 'windows' : 'unix';
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
			/* fall through to the legacy path */
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

	const SECTIONS = [
		{ id: 'check', label: 'Check', accent: 'green' },
		{ id: 'memory', label: 'Memory', accent: 'purple' },
		{ id: 'pipeline', label: 'Pipeline', accent: 'cyan' },
		{ id: 'speed', label: 'Speed', accent: 'orange' },
		{ id: 'taxes', label: 'Taxes', accent: 'pink' },
		{ id: 'tools', label: 'Tools', accent: 'purple' },
		{ id: 'registry', label: 'Registry', accent: 'green' }
	];

	let activeSection = $state('check');

	function jump(id: string) {
		activeSection = id;
	}

	onMount(() => {
		const targets = SECTIONS.map((s) => document.getElementById(s.id)).filter(
			(el): el is HTMLElement => el !== null
		);
		if (targets.length === 0) return;
		// Scrollspy follows the section whose top has passed the upper third
		// of the viewport. The previous IntersectionObserver picked whichever
		// tall section won the ratio test, so Speed never held focus.
		let ticking = false;
		const update = () => {
			ticking = false;
			const line = window.scrollY + window.innerHeight * 0.35;
			let current = targets[0].id;
			for (const t of targets) {
				if (t.getBoundingClientRect().top + window.scrollY <= line) current = t.id;
			}
			activeSection = current;
		};
		const request = () => {
			if (!ticking) {
				ticking = true;
				requestAnimationFrame(update);
			}
		};
		update();
		window.addEventListener('scroll', request, { passive: true });
		window.addEventListener('resize', request);
		return () => {
			window.removeEventListener('scroll', request);
			window.removeEventListener('resize', request);
		};
	});

	const FACTS = [
		{ figure: '0', label: 'collector pauses', note: 'Scopes free on the releasing thread.' },
		{ figure: '2', label: 'reference types', note: 'ARC for ownership, GenRef for back-edges.' },
		{ figure: '3', label: 'cycle lints', note: 'W104, W108, W109 report what stays open.' }
	];
</script>

<svelte:head>
	<title>Rasmalai | Memory safety without a garbage collector</title>
	<meta
		name="description"
		content="A systems language with ARC and GenRef memory, four backends from one LIR, and one rnx binary for check, test, build, and publish."
	/>
</svelte:head>

<div class="home">
	<BackgroundFX />
	<header class="mast">
		<div class="shell mast-grid">
			<div class="mast-main">
				<h1 class="mast-title">
					Memory safety without a <span class="mast-title-accent">garbage collector</span>
				</h1>
				<p class="mast-lede">
					Scopes free what they own, so there is no collector pause and no lifetime to
					annotate. Shared ownership is ARC, back-edges use GenRef, and one binary checks,
					runs, and links the same source for four backends.
				</p>

				<div class="mast-actions">
					<div class="install">
						<div class="install-line">
							<span class="prompt" aria-hidden="true">{os === 'windows' ? '>' : '$'}</span>
							<code>{installCmd}</code>
						</div>
						<div class="install-foot">
							<div class="os-switch" role="group" aria-label="Operating system">
								<button
									onclick={() => (os = 'unix')}
									aria-pressed={os === 'unix'}
									class:os-active={os === 'unix'}
								>
									Linux / macOS
								</button>
								<button
									onclick={() => (os = 'windows')}
									aria-pressed={os === 'windows'}
									class:os-active={os === 'windows'}
								>
									Windows
								</button>
							</div>
							<button onclick={copyInstall} class="copy" aria-label="Copy install command">
								{copied ? 'copied' : 'copy'}
							</button>
						</div>
					</div>
					<a href="/playground" class="mast-link">Open the playground</a>
				</div>
			</div>

			<HeapField />
		</div>
	</header>

	<div class="shell report">
		<nav class="report-index" aria-label="Sections">
			<p class="report-index-label">On this page</p>
			{#each SECTIONS as s, i}
				<a
					href="#{s.id}"
					onclick={() => jump(s.id)}
					class="report-index-link"
					style="--accent: rgb(var(--aura-{s.accent}))"
					aria-current={activeSection === s.id ? 'true' : undefined}
				>
					<span class="report-index-num">{String(i + 1).padStart(2, '0')}</span>
					<span>{s.label}</span>
				</a>
			{/each}
		</nav>

		<div class="report-body">
			<section id="check" class="block" style="--accent: rgb(var(--aura-green))">
				<header class="block-head">
					<h2 class="block-title">
						The type checker <span class="title-accent">runs</span> in your browser
					</h2>
					<p class="block-note">
						This is the real checker, compiled to wasm and running off-thread in your
						browser. Edit the source, press Re-JIT once to load the engine, and the same
						frontend the CLI runs answers in place.
					</p>
				</header>
				<figure class="figure">
					<figcaption class="fig-bar">
						<span class="fig-cap">rnx check, compiled to wasm</span>
						<span class="fig-live"><span class="fig-dot"></span>on demand</span>
					</figcaption>
					<HeroWorkbench />
				</figure>
			</section>

			<section id="memory" class="block" style="--accent: rgb(var(--aura-purple))">
				<header class="block-head">
					<h2 class="block-title">
						Shared ownership is <span class="title-accent">ARC</span>, cycles are explicit
					</h2>
					<p class="block-note">
						Objects are reference counted, and back-edges use GenRef&lt;T&gt;, so a cycle is
						a type you chose. Lints catch own-field closure captures, mutual strong fields,
						and allowed cycles that nothing clears.
					</p>
				</header>
				<div class="split">
					<figure class="figure">
						<figcaption class="fig-bar">
							<span class="fig-cap">scope of fn shade()</span>
						</figcaption>
						<ScopeLedger />
					</figure>
					<dl class="facts">
						{#each FACTS as f}
							<div class="fact">
								<dt class="fact-figure">{f.figure}</dt>
								<dd class="fact-body">
									<span class="fact-label">{f.label}</span>
									<span class="fact-note">{f.note}</span>
								</dd>
							</div>
						{/each}
					</dl>
				</div>
			</section>

			<section id="pipeline" class="block" style="--accent: rgb(var(--aura-cyan))">
				<header class="block-head">
					<h2 class="block-title">
						One intermediate representation, <span class="title-accent">four</span> backends
					</h2>
					<p class="block-note">
						One canonical LIR feeds four backends: an interpreter, a Cranelift JIT, an LLVM
						JIT, and LLVM AOT. The test suite asserts stdout and exit codes match across
						all four.
					</p>
				</header>
				<figure class="figure">
					<figcaption class="fig-bar">
						<span class="fig-cap">src/shade.rnx through the compiler</span>
					</figcaption>
					<Journey />
				</figure>
			</section>

			<section id="speed" class="block" style="--accent: rgb(var(--aura-orange))">
				<header class="block-head">
					<h2 class="block-title">
						Speed measured against <span class="title-accent">other</span> toolchains
					</h2>
					<p class="block-note">
						Every workload is measured against the toolchains that solve it. The plot maps
						runtime against peak RAM or build time, and bottom-left wins. The methodology
						and raw run sit under the chart.
					</p>
				</header>
				<figure class="figure">
					<figcaption class="fig-bar">
						<span class="fig-cap">runtime against peak RAM or build time</span>
					</figcaption>
					<ParetoScatter />
				</figure>
			</section>

			<section id="taxes" class="block" style="--accent: rgb(var(--aura-pink))">
				<header class="block-head">
					<h2 class="block-title">
						What your current toolchain <span class="title-accent">costs</span>
					</h2>
					<p class="block-note">
						Every tool charges a tax. Pick the one you have paid and see what Rasmalai does
						instead.
					</p>
				</header>
				<figure class="figure">
					<figcaption class="fig-bar">
						<span class="fig-cap">cost of the alternatives</span>
					</figcaption>
					<TradeoffSwitchboard />
				</figure>
			</section>

			<section id="tools" class="block" style="--accent: rgb(var(--aura-purple))">
				<header class="block-head">
					<h2 class="block-title">
						One binary for check, test, and <span class="title-accent">build</span>
					</h2>
					<p class="block-note">
						Diagnostics, isolated tests, lint fixes that apply themselves, and native C-ABI
						imports all live behind one binary. Capabilities are proved at compile time, and
						<code>rnx build --lib</code> emits a static library plus header.
					</p>
				</header>
				<figure class="figure">
					<figcaption class="fig-bar">
						<span class="fig-cap">rnx check, test, lint, build</span>
					</figcaption>
					<ToolchainStation />
				</figure>
			</section>

			<section id="registry" class="block block-end" style="--accent: rgb(var(--aura-green))">
				<header class="block-head">
					<h2 class="block-title">
						Every publish is audited, then <span class="title-accent">hashed</span>
					</h2>
					<p class="block-note">
						The server runs the compiler's analyzer over the source tree, rejects
						undeclared capabilities, canonicalizes and chunks it, then names the version
						by digest.
					</p>
				</header>
				<figure class="figure">
					<figcaption class="fig-bar">
						<span class="fig-cap">schematic: one package, four stages</span>
					</figcaption>
					<RegistryLedger />
				</figure>
				<div class="end-links">
					<a href="/packages" class="btn btn-primary">Browse packages</a>
					<a href="/guide/08-modules-and-packages" class="btn btn-ghost">Publish a package</a>
					<a href="/manual" class="btn btn-quiet">Manual</a>
				</div>
			</section>
		</div>
	</div>
</div>

<style>
	.home ::selection {
		background: rgb(var(--aura-purple) / 0.32);
	}

	/* Ambient orbs sit fixed behind everything; keep content above them. */
	.home > header,
	.home > .shell {
		position: relative;
		z-index: 1;
	}

	.mast :where(a, button):focus-visible {
		outline: 2px solid var(--color-accent);
		outline-offset: 2px;
	}

	.mast {
		position: relative;
		isolation: isolate;
		overflow: clip;
		min-height: clamp(660px, 78dvh, 720px);
		padding-block: clamp(56px, 7vw, 96px) clamp(40px, 5vw, 64px);
	}

	.mast-grid {
		position: relative;
		z-index: 1;
		display: grid;
		grid-template-columns: minmax(0, 34rem) minmax(0, 1fr);
		align-items: stretch;
		gap: clamp(28px, 4.4vw, 64px);
	}

	.mast-main {
		max-width: 640px;
		min-width: 0;
	}

	.mast-title {
		margin-top: var(--space-md);
		max-width: 16ch;
		font-family: var(--font-display);
		font-weight: 700;
		font-size: 3rem;
		line-height: 1.02;
		letter-spacing: -0.03em;
		color: var(--color-ink);
		text-wrap: balance;
	}

	.mast-title-accent {
		color: var(--color-accent);
	}

	.mast-lede {
		margin-top: var(--space-lg);
		max-width: 52ch;
		font-size: var(--text-lead);
		line-height: 1.5;
		color: var(--color-muted);
	}

	.mast-actions {
		margin-top: var(--space-2xl);
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--space-lg);
	}

	.install {
		width: min(100%, 470px);
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		background: var(--color-paper-2);
	}

	.install-line {
		display: flex;
		align-items: baseline;
		gap: var(--space-xs);
		padding: var(--space-sm) var(--space-md);
		overflow-x: auto;
		white-space: nowrap;
		font-size: var(--text-xs);
	}

	.prompt {
		color: var(--color-accent);
	}

	.install-line code {
		border: 0;
		border-radius: 0;
		background: none;
		padding: 0;
		font-size: inherit;
		color: var(--color-ink);
	}

	@media (max-width: 1179px) {
		.install-line {
			white-space: normal;
			overflow-wrap: anywhere;
		}
	}

	@media (max-width: 640px) {
		.install-line {
			font-size: var(--text-2xs);
		}
	}

	.install-foot {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--space-sm);
		border-top: 1px solid var(--color-rule);
		padding: var(--space-xs) var(--space-sm);
	}

	.os-switch {
		display: flex;
		align-items: stretch;
		border: 1px solid var(--color-rule);
		border-radius: 999px;
		overflow: hidden;
	}

	.os-switch button {
		padding: 5px var(--space-sm);
		font-size: var(--text-2xs);
		color: var(--color-faint);
		white-space: nowrap;
		transition:
			color var(--dur-micro) ease,
			background-color var(--dur-micro) ease;
	}

	.os-switch button + button {
		border-left: 1px solid var(--color-rule);
	}

	.os-switch button:hover {
		color: var(--color-ink);
	}

	.os-switch .os-active {
		background: rgb(var(--aura-text) / 0.08);
		color: var(--color-ink);
	}

	.copy {
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-xs);
		padding: 5px var(--space-sm);
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-muted);
		white-space: nowrap;
		transition:
			color var(--dur-micro) ease,
			border-color var(--dur-micro) ease;
	}

	.copy:hover {
		border-color: var(--color-accent);
		color: var(--color-ink);
	}

	.copy:active {
		transform: translateY(1px);
	}

	@media (prefers-reduced-motion: reduce) {
		.os-switch button,
		.copy {
			transition: none;
		}

		.copy:active {
			transform: none;
		}
	}

	.mast-link {
		font-size: var(--text-sm);
		color: var(--color-signal);
		text-decoration: underline;
		text-decoration-color: rgb(var(--aura-purple) / 0.4);
		text-underline-offset: 4px;
	}

	.mast-link:hover {
		text-decoration-color: currentColor;
	}

	.report {
		display: grid;
		grid-template-columns: 170px minmax(0, 1fr);
		gap: var(--gap-column);
	}

	.report-index {
		align-self: start;
		position: sticky;
		top: var(--header-h);
		max-height: calc(100dvh - var(--header-h));
		overflow-y: auto;
		overscroll-behavior: contain;
		scrollbar-width: thin;
		padding: var(--space-2xl) 0;
	}

	.report-index-label {
		font-family: var(--font-body);
		font-size: var(--text-2xs);
		color: var(--color-faint);
		padding-left: var(--space-sm);
	}

	.report-index-link {
		display: flex;
		align-items: baseline;
		gap: var(--space-xs);
		border-left: 1px solid var(--color-rule);
		padding: var(--space-2xs) 0 var(--space-2xs) var(--space-sm);
		font-size: var(--text-sm);
		color: var(--color-muted);
		transition:
			color var(--dur-micro) ease,
			border-color var(--dur-micro) ease;
	}

	.report-index-link:hover {
		color: var(--color-ink);
	}

	.report-index-link:hover .report-index-num {
		color: var(--accent, var(--color-accent));
	}

	.report-index-link[aria-current='true'] {
		border-left-color: var(--accent, var(--color-accent));
		color: var(--color-ink);
	}

	.report-index-num {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.report-index-link[aria-current='true'] .report-index-num {
		color: var(--accent, var(--color-accent));
	}

	.report-body {
		min-width: 0;
		padding-bottom: var(--space-3xl);
	}

	.block {
		padding-top: var(--space-4xl);
		scroll-margin-top: calc(var(--header-h) + var(--space-md));
	}

	.block + .block {
		margin-top: var(--space-3xl);
		border-top: 1px solid var(--color-rule);
	}

	.block-head {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 0.72fr);
		gap: var(--space-lg);
		align-items: start;
		margin-bottom: var(--space-2xl);
	}

	.block-title {
		max-width: 22ch;
		font-family: var(--font-display);
		font-weight: 600;
		font-size: clamp(1.8rem, 3vw, 2.5rem);
		line-height: 1.02;
		letter-spacing: -0.032em;
		color: var(--color-ink);
		text-wrap: balance;
	}

	.block-note {
		max-width: 46ch;
		padding-top: 0.4em;
		font-size: var(--text-sm);
		line-height: 1.65;
		color: var(--color-muted);
	}

	.block-note code {
		white-space: nowrap;
		color: var(--color-ink-2);
	}

	.figure {
		min-width: 0;
	}

	.fig-bar {
		display: flex;
		align-items: center;
		gap: var(--space-sm);
		border-bottom: 1px solid var(--color-rule);
		padding-bottom: var(--space-xs);
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.title-accent {
		color: var(--accent, var(--color-accent));
	}

	.fig-cap {
		min-width: 0;
	}

	.fig-live {
		margin-left: auto;
		display: inline-flex;
		align-items: center;
		gap: 5px;
		color: var(--color-muted);
	}

	.fig-dot {
		width: 5px;
		height: 5px;
		border-radius: 999px;
		background: var(--color-signal);
	}

	.split {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 280px;
		gap: var(--gap-column);
		align-items: start;
	}

	.facts {
		display: flex;
		flex-direction: column;
	}

	.fact {
		display: flex;
		align-items: baseline;
		gap: var(--space-md);
		border-bottom: 1px solid var(--color-rule);
		padding: var(--space-md) 0;
	}

	.fact:first-child {
		padding-top: 0;
	}

	.fact-figure {
		font-family: var(--font-display);
		font-weight: 600;
		font-size: clamp(2rem, 2.6vw, 2.6rem);
		line-height: 0.9;
		font-variant-numeric: tabular-nums;
		color: var(--color-ink);
	}

	.fact-body {
		display: flex;
		flex-direction: column;
		gap: 1px;
		min-width: 0;
	}

	.fact-label {
		font-size: var(--text-sm);
		color: var(--color-ink);
	}

	.fact-note {
		font-size: var(--text-xs);
		color: var(--color-faint);
	}

	.end-links {
		display: flex;
		flex-wrap: wrap;
		gap: var(--space-sm);
		margin-top: var(--space-xl);
	}

	@media (max-width: 1199px) {
		.report {
			grid-template-columns: 148px minmax(0, 1fr);
		}
	}

	@media (min-width: 768px) {
		.mast-title {
			font-size: 4.5rem;
		}
	}

	@media (max-width: 1179px) and (min-width: 901px) {
		.mast-grid {
			grid-template-columns: minmax(0, 24rem) minmax(0, 1fr);
			gap: var(--space-xl);
		}

		.mast-title {
			max-width: 18ch;
		}

		.mast-lede {
			font-size: var(--text-md);
		}
	}

	@media (max-width: 900px) {
		.mast-grid {
			grid-template-columns: minmax(0, 1fr);
			gap: var(--space-xl);
			align-items: start;
		}

		.mast-main {
			max-width: none;
		}
	}

	@media (max-width: 1023px) {
		.mast-title {
			max-width: 20ch;
		}

		.report {
			grid-template-columns: minmax(0, 1fr);
			gap: 0;
		}

		.report-index {
			position: static;
			max-height: none;
			overflow: visible;
			display: flex;
			gap: var(--space-md);
			border-bottom: 1px solid var(--color-rule);
			padding: var(--space-md) 0;
			overflow-x: auto;
		}

		.report-index-label {
			display: none;
		}

		.report-index-link {
			border-left: 0;
			padding: 0;
			white-space: nowrap;
		}

		.report-index-link[aria-current='true'] {
			border-left: 0;
			box-shadow: inset 0 -1px 0 var(--accent, var(--color-accent));
		}

		.block-head {
			grid-template-columns: minmax(0, 1fr);
			gap: var(--space-sm);
			margin-bottom: var(--space-lg);
		}

		.block-note {
			padding-top: 0;
		}

		.split {
			grid-template-columns: minmax(0, 1fr);
		}

		.block {
			padding-top: var(--space-2xl);
		}

		.block + .block {
			margin-top: var(--space-2xl);
		}
	}

	@media (max-width: 900px) {
		.mast {
			min-height: 0;
			padding-bottom: var(--space-xl);
		}

		.mast-main {
			max-width: none;
		}
	}
</style>
