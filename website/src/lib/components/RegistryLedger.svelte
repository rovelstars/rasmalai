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
			note: 'First publish. Eight chunks, all new.'
		},
		{
			v: '0.2.0',
			digest: '8b41...77aa',
			fresh: [0],
			note: 'src/window.rnx changed. One new chunk.'
		},
		{
			v: '0.3.0',
			digest: 'c07e...1d93',
			fresh: [5],
			note: 'src/draw.rnx changed. One new chunk.'
		}
	];

	const TREE = ['src/main.rnx', 'src/window.rnx', 'Project.config', 'LICENSE'];

	const CAPS = [
		['fs:read', 'declared'],
		['term:write', 'declared'],
		['net:http', 'missing']
	];

	let active = $state(-1);

	const note = $derived(active >= 0 ? VERSIONS[active].note : '');
</script>

<figure class="ledger">
	<div class="band">
		<p class="band-label">publish</p>
		<div class="band-body">
			<div class="publish-grid">
				<ul class="tree">
					{#each TREE as file}
						<li>{file}</li>
					{/each}
				</ul>
				<div class="declared">
					<p class="declared-label">permissions</p>
					<code>fs:read, term:write</code>
					<p class="flow">source only</p>
				</div>
			</div>
		</div>
	</div>

	<div class="band">
		<p class="band-label">analyse</p>
		<div class="band-body">
			<p class="steps">parse, resolve, analyse</p>
			<ul class="caps">
				{#each CAPS as [name, verdict]}
					<li>
						<code>{name}</code>
						<span class:missing={verdict === 'missing'} class:ok={verdict === 'declared'}>
							{verdict}
						</span>
					</li>
				{/each}
			</ul>
			<p class="flow">fail closed; cached by bytes, imports, ceiling</p>
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
					onclick={() => (active = active === i ? -1 : i)}
					aria-pressed={active === i}
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
						{row.fresh.length === 8 ? '8 stored' : '1 new'}
					</span>
					<code class="ver-digest"><span class="digest-prefix">sha256</span>{row.digest}</code>
				</button>
			{/each}
			<p class="store-key">
				<span class="cell fresh"></span> new
				<span class="cell"></span> shared
			</p>
		</div>
	</div>

	<div class="band">
		<p class="band-label">install</p>
		<div class="band-body">
			<ul class="install">
				<li><code>rnx add</code><span>ceiling</span></li>
				<li><code>chunks</code><span>verified</span></li>
				<li><code>Project.deplock</code><span>pinned</span></li>
			</ul>
		</div>
	</div>

	<figcaption class="registry-note">
		<span aria-live="polite">{note}</span> Versions never change. Unpublish within 72 hours. OCI
		artifacts stay pinned.
	</figcaption>
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
		padding: var(--space-lg) var(--space-md);
		border-right: 1px solid var(--color-rule);
		font-family: var(--font-body);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.band-body {
		display: grid;
		gap: var(--space-sm);
		padding: var(--space-lg);
		min-width: 0;
	}

	.publish-grid {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 15rem);
		gap: var(--space-lg);
		align-items: start;
	}

	.tree {
		display: grid;
		gap: 2px;
		min-width: 0;
	}

	.tree li {
		color: var(--color-ink-2);
	}

	.declared {
		display: grid;
		gap: 2px;
		min-width: 0;
	}

	.declared-label {
		font-family: var(--font-body);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.declared code {
		background: none;
		border: 1px solid var(--color-rule);
		padding: var(--space-xs) var(--space-sm);
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-ink);
		overflow-wrap: anywhere;
	}

	.flow {
		color: var(--color-signal);
	}

	.steps {
		color: var(--color-muted);
	}

	.caps {
		display: grid;
		gap: 2px;
		max-width: 22rem;
	}

	.caps li {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		gap: var(--space-lg);
	}

	.caps code {
		background: none;
		border: 0;
		padding: 0;
		font-family: var(--font-mono);
		color: var(--color-ink);
	}

	.caps .ok {
		color: var(--color-signal);
	}

	.caps .missing {
		color: var(--color-error);
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
		cursor: pointer;
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

		.publish-grid {
			grid-template-columns: minmax(0, 1fr);
			gap: var(--space-sm);
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

		.install li {
			grid-template-columns: minmax(0, 1fr) auto;
		}

		.cell {
			width: 12px;
			height: 12px;
		}
	}
</style>
