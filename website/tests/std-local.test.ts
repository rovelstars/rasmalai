import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { localStdChunk, localStdEntries } from '../src/lib/server/std-local.ts';

describe('local stdlib serving', () => {
	it('presents the checkout in the registry layout', () => {
		const entries = localStdEntries('@std/prelude');
		assert.ok(entries);
		const entry = entries.find((e) => !e.dir && e.name === 'src/lib.rnx');
		assert.ok(entry, 'prelude exposes src/lib.rnx');
		assert.equal(entry.chunks.length, 1);
	});

	it('reads a chunk from the real lib.rnx path', () => {
		const entries = localStdEntries('@std/prelude');
		assert.ok(entries);
		const entry = entries.find((e) => e.name === 'src/lib.rnx');
		assert.ok(entry);
		const bytes = localStdChunk('@std/prelude', entry.chunks[0]!);
		assert.ok(bytes);
		assert.ok(bytes.length > 0);
	});

	it('keeps app submodules at their registry paths', () => {
		const entries = localStdEntries('@std/app');
		assert.ok(entries);
		const entry = entries.find((e) => e.name === 'src/os/platform.rnx');
		assert.ok(entry);
		const bytes = localStdChunk('@std/app', entry.chunks[0]!);
		assert.ok(bytes);
		assert.ok(new TextDecoder().decode(bytes).includes('Platform'));
	});

	it('does not walk wrapper or test build trees', () => {
		const entries = localStdEntries('@std/app');
		assert.ok(entries);
		assert.ok(!entries.some((e) => e.name.startsWith('wrappers/')));
		assert.ok(!entries.some((e) => e.name.startsWith('tests/')));
	});
});
