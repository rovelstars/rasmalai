<script lang="ts">
	import { onMount } from 'svelte';
	import { EditorView, hoverTooltip, keymap, lineNumbers } from '@codemirror/view';
	import { EditorState } from '@codemirror/state';
	import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
	import { autocompletion, type Completion } from '@codemirror/autocomplete';
	import { linter, type Diagnostic as LintDiagnostic } from '@codemirror/lint';
	import { bracketMatching, indentOnInput } from '@codemirror/language';
	import { rnxLanguage, rnxHighlight, computeIndent, dedentForClosing } from '$lib/aura/rnx-mode';
	import { RNX_KEYWORDS, RNX_TYPES } from '$lib/aura/keywords.gen';
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
		'&': { backgroundColor: 'transparent', height: '100%', color: 'rgb(var(--aura-text))' },
		'.cm-content': {
			fontFamily: 'var(--font-mono)',
			fontSize: '13px',
			lineHeight: '1.7',
			paddingTop: '10px',
			paddingBottom: '24px',
			caretColor: 'rgb(var(--aura-purple))'
		},
		'.cm-scroller': { overflow: 'auto', minHeight: '320px' },
		'.cm-gutters': {
			backgroundColor: 'transparent',
			borderRight: '1px solid rgb(var(--aura-line))',
			color: 'rgb(var(--aura-faint))'
		},
		'.cm-activeLine': { backgroundColor: 'rgb(var(--aura-purple) / 0.07)' },
		'.cm-activeLineGutter': { backgroundColor: 'transparent' },
		'&.cm-focused': { outline: 'none' },
		'.cm-selectionBackground, ::selection': { backgroundColor: 'rgb(var(--aura-purple) / 0.3)' },
		'.cm-cursor': { borderLeftColor: 'rgb(var(--aura-purple))' },
		'.cm-tooltip': {
			backgroundColor: 'rgb(var(--aura-surfaceElevated))',
			border: '1px solid rgb(var(--aura-lineStrong))',
			borderRadius: '4px',
			color: 'rgb(var(--aura-text))',
			fontFamily: 'var(--font-mono)',
			fontSize: '12px'
		},
		'.cm-tooltip-autocomplete ul li[aria-selected]': { backgroundColor: 'rgb(var(--aura-purple) / 0.22)' },
		'.cm-diagnostic-error': { borderBottom: '2px wavy rgb(var(--aura-red))' },
		'.cm-diagnostic-warning': { borderBottom: '2px wavy rgb(var(--aura-orange))' }
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

	function wordPrefix(context: { pos: number; state: EditorState }): { prefix: string; from: number } {
		const line = context.state.doc.lineAt(context.pos);
		const before = line.text.slice(0, context.pos - line.from);
		const m = /[A-Za-z_][A-Za-z0-9_]*$/.exec(before);
		return {
			prefix: (m ? m[0] : '').toLowerCase(),
			from: m && m.index !== undefined ? line.from + m.index : context.pos
		};
	}

	// Word-based fallback for when the compiler cannot complete: the document
	// has a parse error (the normal state mid-keystroke) or the engine is not
	// downloaded yet. Keywords and types mirror the compiler oracles; the rest
	// comes from words already in the file. Never guesswork from another
	// language: this editor has no JS machinery installed.
	function localOptions(doc: string, prefix: string): Completion[] {
		const seen = new Set<string>();
		const out: Completion[] = [];
		const push = (label: string, type: Completion['type']): void => {
			const key = label.toLowerCase();
			if (seen.has(key) || (prefix && !key.startsWith(prefix))) return;
			seen.add(key);
			out.push({ label, type });
		};
		for (const k of RNX_KEYWORDS) push(k, 'keyword');
		for (const t of RNX_TYPES) push(t, 'class');
		for (const w of doc.matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)) {
			if (out.length >= 64) break;
			if (!RNX_KEYWORDS.has(w[0]) && !RNX_TYPES.has(w[0])) push(w[0], 'variable');
		}
		return out.slice(0, 64);
	}

	async function completeAt(context: {
		pos: number;
		state: EditorState;
		explicit: boolean;
	}): Promise<{ from: number; options: Completion[] } | null> {
		const { prefix, from } = wordPrefix(context);
		if (!engine.loaded) {
			const options = localOptions(context.state.doc.toString(), prefix);
			return options.length > 0 ? { from, options } : null;
		}
		const doc = context.state.doc.toString();
		const { line, character } = offsetToLineCol(doc, context.pos);
		let items;
		try {
			items = await engine.complete(doc, line, character);
		} catch {
			const options = localOptions(doc, prefix);
			return options.length > 0 ? { from, options } : null;
		}
		if (items.length === 0) return null;
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
							sig.style.color = 'rgb(var(--aura-purple))';
							dom.appendChild(sig);
							if (info.docs) {
								const docs = document.createElement('div');
								docs.textContent = info.docs;
								docs.style.color = 'rgb(var(--aura-muted))';
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
