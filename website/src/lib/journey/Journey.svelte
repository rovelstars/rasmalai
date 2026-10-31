<script lang="ts">
	import { STAGES } from './stages';
	import Artifact from './Artifact.svelte';

	let active = $state(0);

	const acts = $derived(
		STAGES.reduce<{ act: string; from: number; to: number }[]>((out, s, i) => {
			const last = out[out.length - 1];
			if (last && last.act === s.act) last.to = i;
			else out.push({ act: s.act, from: i, to: i });
			return out;
		}, [])
	);

	let current = $derived(STAGES[active]);

	const PANEL_ID = 'compiler-stage-panel';

	function tabId(index: number): string {
		return `compiler-stage-tab-${STAGES[index].id}`;
	}

	function onTabKeydown(event: KeyboardEvent, index: number) {
		const last = STAGES.length - 1;
		let next: number;
		switch (event.key) {
			case 'ArrowRight':
			case 'ArrowDown':
				next = index === last ? 0 : index + 1;
				break;
			case 'ArrowLeft':
			case 'ArrowUp':
				next = index === 0 ? last : index - 1;
				break;
			case 'Home':
				next = 0;
				break;
			case 'End':
				next = last;
				break;
			default:
				return;
		}
		event.preventDefault();
		active = next;
		const tabs = (event.currentTarget as HTMLElement)
			.closest('.pipeline-rail')
			?.querySelectorAll<HTMLButtonElement>('[role="tab"]');
		tabs?.[next]?.focus();
	}
</script>

<div class="pipeline">
	<div class="pipeline-rail" role="tablist" aria-label="Compiler stages">
		{#each acts as group}
			<p class="pipeline-act" role="presentation">{group.act}</p>
			{#each STAGES.slice(group.from, group.to + 1) as s, i}
				{@const index = group.from + i}
				<button
					id={tabId(index)}
					role="tab"
					aria-selected={active === index}
					aria-controls={PANEL_ID}
					tabindex={active === index ? 0 : -1}
					class="pipeline-tab"
					onclick={() => (active = index)}
					onkeydown={(event) => onTabKeydown(event, index)}
				>
					<span class="pipeline-num">{s.kicker.slice(0, 2)}</span>
					<span class="pipeline-name">{s.kicker.slice(5)}</span>
				</button>
			{/each}
		{/each}
	</div>

	<div class="pipeline-panel" role="tabpanel" id={PANEL_ID} aria-labelledby={tabId(active)}>
		<div class="pipeline-copy">
			<h3 class="pipeline-title">{current.title}</h3>
			<p class="pipeline-body">{current.body}</p>
		</div>
		<div class="pipeline-art">
			<Artifact def={current.artifact} active={true} motionOK={false} />
		</div>
	</div>
</div>

<style>
	.pipeline {
		display: grid;
		grid-template-columns: 196px minmax(0, 1fr);
		gap: var(--gap-column);
		/* The caption rule is shared with every figure; without top air the
		act label and title sit clamped against it. */
		padding-top: var(--space-lg);
	}

	.pipeline-rail {
		display: flex;
		flex-direction: column;
		align-items: stretch;
	}

	.pipeline-act {
		margin-top: var(--space-lg);
		padding-left: var(--space-sm);
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.pipeline-act:first-child {
		margin-top: 0;
	}

	.pipeline-tab {
		display: flex;
		align-items: baseline;
		gap: var(--space-xs);
		border-left: 1px solid var(--color-rule);
		padding: var(--space-2xs) 0 var(--space-2xs) var(--space-sm);
		text-align: left;
		font-size: var(--text-sm);
		color: var(--color-muted);
		transition:
			color var(--dur-micro) ease,
			border-color var(--dur-micro) ease;
	}

	.pipeline-tab:hover {
		border-left-color: var(--color-rule-strong);
		color: var(--color-ink);
	}

	.pipeline-tab[aria-selected='true'] {
		border-left-color: var(--color-accent);
		color: var(--color-ink);
	}

	.pipeline-num {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.pipeline-tab[aria-selected='true'] .pipeline-num {
		color: var(--color-accent);
	}

	.pipeline-panel {
		display: grid;
		grid-template-columns: minmax(0, 0.72fr) minmax(0, 1fr);
		gap: var(--gap-column);
		align-items: start;
		min-width: 0;
	}

	.pipeline-copy {
		min-width: 0;
	}

	.pipeline-title {
		font-family: var(--font-display);
		font-weight: 600;
		font-size: clamp(1.5rem, 2vw, 1.9rem);
		line-height: 1.1;
		letter-spacing: -0.025em;
		color: var(--color-ink);
	}

	.pipeline-body {
		margin-top: var(--space-sm);
		max-width: 40ch;
		font-size: var(--text-sm);
		line-height: 1.65;
		color: var(--color-muted);
	}

	.pipeline-art {
		min-width: 0;
	}

	@media (max-width: 1023px) {
		.pipeline {
			grid-template-columns: minmax(0, 1fr);
			gap: var(--space-lg);
		}

		.pipeline-rail {
			flex-direction: row;
			flex-wrap: wrap;
			gap: 0 var(--space-md);
			border-top: 1px solid var(--color-rule);
			padding-top: var(--space-xs);
		}

		.pipeline-act {
			width: 100%;
			margin-top: var(--space-sm);
			padding-left: 0;
		}

		.pipeline-tab {
			padding: 2px 0;
			border-left: 0;
		}

		.pipeline-tab[aria-selected='true'] {
			box-shadow: inset 0 -1px 0 var(--color-accent);
		}

		.pipeline-panel {
			grid-template-columns: minmax(0, 1fr);
			gap: var(--space-lg);
		}
	}
</style>
