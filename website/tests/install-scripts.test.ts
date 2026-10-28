import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = join(import.meta.dirname, '..');
const REPO = join(ROOT, '..');

describe('install scripts', () => {
	it('repo install.sh mirrors the served installer', () => {
		const served = readFileSync(join(ROOT, 'static', 'install.sh'), 'utf8');
		const mirror = readFileSync(join(REPO, 'install.sh'), 'utf8');
		assert.equal(mirror, served);
	});

	it('served install.sh accepts a pinned version', () => {
		const sh = readFileSync(join(ROOT, 'static', 'install.sh'), 'utf8');
		assert.match(sh, /usage: install\.sh \[version\]/);
		assert.match(sh, /VERSION="\$1"/);
		assert.match(sh, /releases\/download\/\$VERSION/);
	});

	for (const [file, seed] of [
		['install.sh', '"$BIN" download'],
		['install.ps1', '& $Bin download']
	] as const) {
		it(`${file} seeds the stdlib cache and stays offline-tolerant`, () => {
			const text = readFileSync(join(ROOT, 'static', file), 'utf8');
			assert.ok(text.includes(seed), `${file} runs rnx download after install`);
			assert.match(text, /rnx download.*later/, `${file} names the retry when seeding fails`);
		});
	}
});
