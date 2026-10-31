<script lang="ts">
	interface Version {
		v: string;
		digest: string;
		fresh: number[];
		note: string;
	}

	const VERSIONS: Version[] = [
		{
			v: '0.1.0',
			digest: '3f9a...c21e',
			fresh: [0, 1, 2, 3, 4, 5, 6, 7],
			note: 'First publish. Every chunk is new, so all eight are stored.'
		},
		{
			v: '0.2.0',
			digest: '8b41...77aa',
			fresh: [0],
			note: 'src/window.rnx changed. One new chunk; the other seven were already stored.'
		},
		{
			v: '0.3.0',
			digest: 'c07e...1d93',
			fresh: [5],
			note: 'src/draw.rnx changed. One new chunk again: twenty-four references, ten chunks on disk.'
		}
	];

	const TREE = [
		['src/window.rnx', '1.9 kB'],
		['src/main.rnx', '1.2 kB'],
		['src/layout.rnx', '0.9 kB'],
		['src/input.rnx', '0.8 kB'],
		['src/theme.rnx', '0.7 kB'],
		['src/draw.rnx', '0.6 kB'],
		['README.md', '2.1 kB'],
		['Project.config', '0.4 kB']
	];

	const INSTALL = [
		['rnx add @acme/window', 'pinned 0.3.0'],
		['Project.deplock', 'checksum c07e...1d93'],
		['chunks', '8 of 8 verified'],
		['.rnx-integrity', 'written']
	];

	let active = $state(-1);

	const note = $derived(
		active >= 0 ? VERSIONS[active].note : 'Twenty-four chunk references, ten chunks stored.'
	);
</script>

<figure class="ledger">
	<div class="band">
		<p class="band-label">publish</p>
		<div class="band-body">
			<ul class="tree">
				{#each TREE as [file, size]}
					<li><span>{file}</span><span class="dim">{size}</span></li>
				{/each}
			</ul>
			<p class="flow">canonicalize, chunk, sha256</p>
		</div>
	</div>

	<div class="band">
		<p class="band-label">store</p>
		<div class="band-body">
			{#each VERSIONS as row, i}
				<button
					class="ver"
					class:is-active={active === i}
					class:is-dim={active >= 0 && active !== i}
					onpointerenter={() => (active = i)}
					onpointerleave={() => (active = -1)}
					onfocus={() => (active = i)}
					onblur={() => (active = -1)}
					aria-label="{row.v}, {row.fresh.length === 8
						? 'all eight chunks new'
						: 'one new chunk'}"
				>
					<span class="ver-name">{row.v}</span>
					<span class="cells" aria-hidden="true">
						{#each { length: 8 } as _, c}
							<span class="cell" class:fresh={row.fresh.includes(c)}></span>
						{/each}
					</span>
					<span class="ver-count">
						{row.fresh.length === 8 ? '8 stored' : '1 new, 7 shared'}
					</span>
					<code class="ver-digest"><span class="digest-prefix">sha256</span>{row.digest}</code>
				</button>
			{/each}
			<p class="store-key">
				<span class="cell fresh"></span> added by this version
				<span class="cell"></span> already stored
			</p>
		</div>
	</div>

	<div class="band">
		<p class="band-label">install</p>
		<div class="band-body">
			<ul class="install">
				{#each INSTALL as [left, right]}
					<li><code>{left}</code><span>{right}</span></li>
				{/each}
			</ul>
			<p class="aside">
				<code>rnx add</code> prints the package's permission ceiling before it writes the
				lockfile.
			</p>
		</div>
	</div>

	<figcaption class="registry-note">{note}</figcaption>
</figure>

<style>
	.ledger {
		margin: 0;
		border: 1px solid var(--color-rule);
		background: var(--color-paper-2);
		font-family: var(--font-mono);
		font-size: var(--text-xs);
	}

	.band {
		display: grid;
		grid-template-columns: 7rem minmax(0, 1fr);
		gap: 0;
		border-bottom: 1px solid var(--color-rule);
	}

	.band:last-of-type {
		border-bottom: 0;
	}

	.band-label {
		padding: var(--space-md);
		border-right: 1px solid var(--color-rule);
		font-size: var(--text-2xs);
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--color-faint);
	}

	.band-body {
		display: grid;
		gap: var(--space-sm);
		padding: var(--space-md);
		min-width: 0;
	}

	.tree {
		display: grid;
		gap: 2px;
		max-width: 30rem;
	}

	.tree li {
		display: flex;
		justify-content: space-between;
		gap: var(--space-lg);
		color: var(--color-ink-2);
	}

	.tree .dim {
		color: var(--color-faint);
	}

	.flow {
		color: var(--color-signal);
	}

	.ver {
		display: grid;
		grid-template-columns: 4.5rem auto minmax(7rem, 1fr) 9rem;
		align-items: center;
		gap: var(--space-md);
		width: 100%;
		padding: 2px 0;
		border: 0;
		background: none;
		color: var(--color-ink-2);
		font: inherit;
		text-align: left;
		cursor: default;
		transition: opacity 200ms ease, color 200ms ease;
	}

	.ver.is-active {
		color: var(--color-ink);
	}

	.ver.is-dim {
		opacity: 0.5;
	}

	.ver:focus-visible {
		outline: 2px solid var(--color-accent);
		outline-offset: 2px;
	}

	.ver-name {
		color: var(--color-ink);
	}

	.cells {
		display: flex;
		gap: 3px;
	}

	.cell {
		width: 14px;
		height: 14px;
		background: rgb(var(--aura-text) / 0.22);
		transition: background-color 200ms ease;
	}

	.cell.fresh {
		background: rgb(var(--aura-green) / 0.75);
	}

	.ver-count {
		color: var(--color-faint);
		white-space: nowrap;
	}

	.ver-digest {
		color: var(--color-muted);
		text-align: right;
		white-space: nowrap;
		background: none;
		border: 0;
		padding: 0;
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
	}

	.digest-prefix {
		margin-right: var(--space-sm);
		color: var(--color-faint);
	}

	.store-key {
		display: flex;
		align-items: center;
		gap: var(--space-xs);
		margin-top: var(--space-2xs);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.store-key .cell {
		width: 10px;
		height: 10px;
		margin-left: var(--space-sm);
	}

	.store-key .cell:first-child {
		margin-left: 0;
	}

	.install {
		display: grid;
		gap: 3px;
	}

	.install li {
		display: grid;
		grid-template-columns: minmax(0, 16rem) minmax(0, 1fr);
		gap: var(--space-md);
	}

	.install code {
		background: none;
		border: 0;
		padding: 0;
		font-family: var(--font-mono);
		color: var(--color-ink);
	}

	.install span {
		color: var(--color-signal);
	}

	.aside {
		color: var(--color-muted);
		font-family: var(--font-body);
		font-size: var(--text-xs);
	}

	.aside code {
		background: none;
		border: 0;
		padding: 0;
		font-family: var(--font-mono);
		font-size: 0.95em;
		color: var(--color-ink);
	}

	.registry-note {
		padding: var(--space-sm) var(--space-md);
		border-top: 1px solid var(--color-rule);
		font-family: var(--font-body);
		font-size: var(--text-xs);
		color: var(--color-muted);
	}

	@media (max-width: 760px) {
		.band {
			grid-template-columns: minmax(0, 1fr);
		}

		.band-label {
			border-right: 0;
			border-bottom: 1px solid var(--color-rule);
			padding: var(--space-xs) var(--space-md);
		}

		.ver {
			grid-template-columns: 4rem auto minmax(0, 1fr);
			row-gap: var(--space-2xs);
		}

		.ver-count {
			text-align: right;
		}

		.ver-digest {
			grid-column: 1 / -1;
			text-align: left;
		}

		.cell {
			width: 12px;
			height: 12px;
		}
	}
</style>
