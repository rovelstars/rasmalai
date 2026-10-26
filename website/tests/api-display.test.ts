import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { displayModuleName, isGeneratedModule } from '../src/lib/docs/api.ts';

describe('generated module display names', () => {
	it('keeps plain module names untouched', () => {
		assert.equal(isGeneratedModule('sqlite-rnx'), false);
		assert.equal(displayModuleName('sqlite-rnx'), 'sqlite-rnx');
		assert.equal(isGeneratedModule('std.fs'), false);
		assert.equal(displayModuleName('std.fs'), 'std.fs');
	});

	it('strips a single leading gen segment', () => {
		assert.equal(isGeneratedModule('gen.sqlite3'), true);
		assert.equal(displayModuleName('gen.sqlite3'), 'sqlite3');
		assert.equal(isGeneratedModule('gen.bindgen'), true);
		assert.equal(displayModuleName('gen.bindgen'), 'bindgen');
	});

	it('does not treat a bare gen name as generated', () => {
		assert.equal(isGeneratedModule('gen'), false);
		assert.equal(displayModuleName('gen'), 'gen');
		assert.equal(isGeneratedModule('gen.'), false);
		assert.equal(displayModuleName('gen.'), 'gen.');
	});

	it('leaves deeper paths intact', () => {
		assert.equal(displayModuleName('gen.nested.mod'), 'nested.mod');
		assert.equal(displayModuleName('src.gen.sqlite3'), 'src.gen.sqlite3');
	});
});
