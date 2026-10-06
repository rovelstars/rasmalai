import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = join(import.meta.dirname, '..', 'src');

function tsFiles(dir: string): string[] {
	const out: string[] = [];
	for (const e of readdirSync(dir, { withFileTypes: true })) {
		const p = join(dir, e.name);
		if (e.isDirectory()) out.push(...tsFiles(p));
		else if (e.name.endsWith('.ts') || e.name.endsWith('.svelte')) out.push(p);
	}
	return out;
}

describe('workerd safety', () => {
	it('no static node: imports in routes or server lib (dynamic import dev-only code instead)', () => {
		const bad: string[] = [];
		for (const f of [...tsFiles(join(ROOT, 'routes')), ...tsFiles(join(ROOT, 'lib', 'server'))]) {
			if (f.endsWith('lib/server/std-local.ts')) continue;
			const text = readFileSync(f, 'utf8');
			for (const line of text.split('\n')) {
				const t = line.trim();
				if (t.startsWith('import ') && /from ['"]node:/.test(t)) bad.push(`${f}: ${t}`);
			}
		}
		assert.deepEqual(bad, []);
	});

	it('std-local stays out of the static bundle graph', () => {
		const text = readFileSync(join(ROOT, 'routes', 'packages', '[...slug]', '+page.server.ts'), 'utf8');
		assert.ok(!text.includes("from '$lib/server/std-local'"));
		assert.ok(text.includes("await import('$lib/server/std-local')"));
		assert.ok(statSync(join(ROOT, 'lib', 'server', 'std-local.ts')).isFile());
	});
});
