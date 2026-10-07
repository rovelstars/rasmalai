<script lang="ts">
	import { onMount } from 'svelte';
	import { EditorView, hoverTooltip, keymap, lineNumbers } from '@codemirror/view';
	import { EditorState } from '@codemirror/state';
	import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
	import { autocompletion, type Completion } from '@codemirror/autocomplete';
	import { linter, type Diagnostic as LintDiagnostic } from '@codemirror/lint';
	import { bracketMatching, indentOnInput } from '@codemirror/language';
	import { rnxLanguage, rnxHighlight, computeIndent, dedentForClosing } from '$lib/aura/rnx-mode';
	import { engine } from '$lib/playground/engine.svelte';

	let {
		value = $bindable(''),
		onRun,
		filePath = 'main.rnx',
		getProject
	}: {
		value: string;
		onRun: () => void;
		filePath?: string;
		getProject?: () => { json: string; entry: string } | null;
	} = $props();

	let host: HTMLDivElement | null = $state(null);
	let view: EditorView | null = null;
	let syncing = false;

	function offsetToLineCol(doc: string, offset: number): { line: number; character: number } {
		const clamped = Math.max(0, Math.min(offset, doc.length));
		const head = doc.slice(0, clamped);
		const line = head.split('\n').length - 1;
		const lastNl = head.lastIndexOf('\n');
		return { line, character: clamped - lastNl - 1 };
	}

	function kindToType(kind: number): Completion['type'] {
		if (kind === 14) return 'keyword';
		if (kind === 7 || kind === 22) return 'class';
		if (kind === 6) return 'function';
		if (kind === 5) return 'variable';
		return 'text';
	}

	const auraEditorTheme = EditorView.theme({
		'&': { backgroundColor: 'transparent', height: '100%' },
		'.cm-content': {
			fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
			fontSize: '13px',
			lineHeight: '1.625',
			caretColor: '#a277ff'
		},
		'.cm-scroller': { overflow: 'auto', minHeight: '320px' },
		'.cm-gutters': {
			backgroundColor: 'transparent',
			borderRight: '1px solid var(--aura-border, #2b2a35)',
			color: 'var(--aura-muted, #6e6c7e)'
		},
		'.cm-activeLine': { backgroundColor: 'rgba(162, 119, 255, 0.06)' },
		'.cm-activeLineGutter': { backgroundColor: 'transparent' },
		'&.cm-focused': { outline: 'none' },
		'.cm-selectionBackground, ::selection': { backgroundColor: 'rgba(162, 119, 255, 0.3)' },
		'.cm-tooltip': {
			backgroundColor: '#1c1b22',
			border: '1px solid #2b2a35',
			fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
			fontSize: '12px'
		},
		'.cm-tooltip-autocomplete ul li[aria-selected]': { backgroundColor: 'rgba(162, 119, 255, 0.25)' },
		'.cm-diagnostic-error': { borderBottom: '2px wavy #ff5c5c' },
		'.cm-diagnostic-warning': { borderBottom: '2px wavy #ffb224' }
	});

	function electricEnter(target: EditorView): boolean {
		const pos = target.state.selection.main.head;
		const line = target.state.doc.lineAt(pos);
		const before = line.text.slice(0, pos - line.from);
		const after = line.text.slice(pos - line.from);
		const indent = computeIndent(before);
		let insert = `\n${indent}`;
		let cursor = pos + 1 + indent.length;
		if (/[{[(]\s*$/.test(before) && /^\s*[}\])]/.test(after)) {
			const ded = dedentForClosing(indent);
			insert = `\n${indent}\n${ded}`;
		}
		target.dispatch({ changes: { from: pos, insert }, selection: { anchor: cursor } });
		return true;
	}

	async function completeAt(context: {
		pos: number;
		state: EditorState;
		explicit: boolean;
	}): Promise<{ from: number; options: Completion[] } | null> {
		if (!engine.loaded) return null;
		const doc = context.state.doc.toString();
		const { line, character } = offsetToLineCol(doc, context.pos);
		const lineText = context.state.doc.lineAt(context.pos).text;
		const m = /[A-Za-z_][A-Za-z0-9_]*$/.exec(lineText.slice(0, context.pos - context.state.doc.lineAt(context.pos).from));
		const from = m && m.index !== undefined ? context.state.doc.lineAt(context.pos).from + m.index : context.pos;
		let items;
		try {
			items = await engine.complete(doc, line, character);
		} catch {
			return null;
		}
		if (items.length === 0) return null;
		const prefix = m ? m[0].toLowerCase() : '';
		const options: Completion[] = items
			.filter((it) => (prefix ? it.label.toLowerCase().startsWith(prefix) : true))
			.slice(0, 64)
			.map((it) => ({
				label: it.label,
				detail: it.detail,
				apply: it.insertText,
				type: kindToType(it.kind)
			}));
		if (options.length === 0) return null;
		return { from, options };
	}

	onMount(() => {
		if (!host) return;
		const state = EditorState.create({
			doc: value,
			extensions: [
				lineNumbers(),
				history(),
				bracketMatching(),
				indentOnInput(),
				rnxLanguage,
				rnxHighlight(),
				auraEditorTheme,
				EditorView.lineWrapping,
				keymap.of([
					{ key: 'Mod-Enter', run: () => (onRun(), true) },
					{ key: 'Enter', run: electricEnter },
					indentWithTab,
					...defaultKeymap,
					...historyKeymap
				]),
				autocompletion({ override: [completeAt] }),
				hoverTooltip(async (innerView, pos) => {
					if (!engine.loaded) return null;
					const doc = innerView.state.doc.toString();
					const { line, character } = offsetToLineCol(doc, pos);
					let info;
					try {
						info = await engine.hover(doc, line, character);
					} catch {
						return null;
					}
					if (!info) return null;
					const len = doc.length;
					const from = Math.max(0, Math.min(info.start, len));
					const to = Math.max(from, Math.min(info.end, len));
					return {
						pos,
						create() {
							const dom = document.createElement('div');
							dom.className = 'px-2 py-1';
							const sig = document.createElement('div');
							sig.textContent = info.signature;
							sig.style.color = '#a277ff';
							dom.appendChild(sig);
							if (info.docs) {
								const docs = document.createElement('div');
								docs.textContent = info.docs;
								docs.style.color = '#b8b6c4';
								dom.appendChild(docs);
							}
							return { dom };
						}
					};
				}),
				linter(async (innerView) => {
					if (!engine.loaded) return [];
					const doc = innerView.state.doc.toString();
					const out: LintDiagnostic[] = [];
					try {
						const proj = getProject?.() ?? null;
						const diags = proj
							? await engine.diagProject(proj.json, proj.entry)
							: await engine.diagJson(doc);
						const len = doc.length;
						for (const d of diags) {
							if (proj && d.file !== filePath) continue;
							const from = Math.max(0, Math.min(d.start, len));
							const to = Math.max(from, Math.min(d.end, len));
							if (from === to) continue;
							out.push({
								from,
								to,
								severity: d.severity === 'warning' ? 'warning' : 'error',
								message: `${d.code}: ${d.message}`
							});
						}
					} catch {
						return [];
					}
					return out;
				}),
				EditorView.updateListener.of((update) => {
					if (!update.docChanged || syncing) return;
					value = update.state.doc.toString();
				})
			]
		});
		view = new EditorView({ state, parent: host });
		return () => {
			view?.destroy();
			view = null;
		};
	});

	$effect(() => {
		const current = view?.state.doc.toString();
		if (view && current !== value) {
			syncing = true;
			view.dispatch({ changes: { from: 0, to: current?.length ?? 0, insert: value } });
			syncing = false;
		}
	});

	export async function formatDocument(): Promise<void> {
		if (!view) return;
		const doc = view.state.doc.toString();
		const out = await engine.formatSource(doc);
		if (out !== doc) {
			view.dispatch({ changes: { from: 0, to: doc.length, insert: out } });
		}
	}

	export function focusEditor(): void {
		view?.focus();
	}
</script>

<div bind:this={host} class="min-h-[320px] font-mono text-[13px]" aria-label="Rasmalai source editor"></div>
