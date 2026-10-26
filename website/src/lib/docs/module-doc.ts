import type { DocModule } from '$lib/docs/api';

export function firstExport(mod: DocModule | null): string | null {
	if (!mod) return null;
	const c = mod.classes[0]?.name;
	if (c) return c;
	const e = mod.enums[0]?.name;
	if (e) return e;
	const f = mod.functions[0]?.name;
	if (f) return f;
	const k = (mod as { constants?: Array<{ name?: string }> }).constants?.[0]?.name;
	if (k) return k;
	return null;
}

export function genericSnippet(full: string, mod: DocModule | null): string {
	const sym = firstExport(mod);
	if (sym) return `import { ${sym} } from "${full}";`;
	return `// see the API reference for the import list`;
}

export function overviewMarkdown(full: string, mod: DocModule | null, description: string): string {
	const body = mod && mod.docs.description ? mod.docs.description : description;
	const lines = [
		`# ${full} overview`,
		'',
		body,
		'',
		'Continue with [Getting Started](getting-started) or jump to the [API Reference](api).'
	];
	return lines.join('\n');
}

export function gettingStartedMarkdown(
	full: string,
	opts: { importLine: string; snippet: string; installed: boolean }
): string {
	return [
		`# ${full} getting started`,
		'',
		opts.installed
			? 'No install step: the module ships inside the compiler. Import what you need:'
			: `Add \`${full}\` to your manifest dependencies, then import what you need:`,
		'',
		'```rnx',
		opts.importLine,
		'```',
		'',
		'## Quickstart',
		'',
		'Copy this into a file or open it straight in the playground:',
		'',
		'```rnx',
		opts.snippet,
		'```'
	].join('\n');
}

export function moduleFor(
	mods: DocModule[],
	name: string
): DocModule | null {
	return mods.find((m) => m.name === name) ?? mods[0] ?? null;
}
