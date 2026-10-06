import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { ogImageKey, ogImageFor, DEFAULT_OG_IMAGE } from '../src/lib/docs/og.ts';

const MANIFEST = {
	home: 'og-banner.png',
	'guide/01-introduction': 'og-guide-01-introduction.png',
	'manual/18-packages-and-registries': 'og-manual-18-packages-and-registries.png',
	'std/fs': 'og-std-fs.png',
	'pkg/@std/fs': 'og-pkg-std-fs.png',
	'pkg/lodash': 'og-pkg-lodash.png'
};

describe('ogImageKey', () => {
	it('maps home and unknown paths', () => {
		assert.equal(ogImageKey('/'), 'home');
		assert.equal(ogImageKey('/playground'), null);
		assert.equal(ogImageKey('/packages'), null);
	});

	it('maps guide and manual sections', () => {
		assert.equal(ogImageKey('/guide/01-introduction'), 'guide/01-introduction');
		assert.equal(ogImageKey('/manual/18-packages-and-registries'), 'manual/18-packages-and-registries');
	});

	it('maps std module docs', () => {
		assert.equal(ogImageKey('/docs/@std/fs/overview'), 'std/fs');
		assert.equal(ogImageKey('/docs/@std/fs/api'), 'std/fs');
	});

	it('maps scoped, unscoped, and versioned package pages', () => {
		assert.equal(ogImageKey('/packages/@std/fs'), 'pkg/@std/fs');
		assert.equal(ogImageKey('/packages/@std/fs@0.1.0'), 'pkg/@std/fs');
		assert.equal(ogImageKey('/packages/lodash'), 'pkg/lodash');
		assert.equal(ogImageKey('/packages/lodash@4.17.21?tab=code'), 'pkg/lodash');
	});
});

describe('ogImageFor', () => {
	it('resolves through the manifest with default fallback', () => {
		assert.equal(ogImageFor('/', MANIFEST), '/og-banner.png');
		assert.equal(ogImageFor('/guide/01-introduction', MANIFEST), '/og-guide-01-introduction.png');
		assert.equal(ogImageFor('/packages/@std/fs', MANIFEST), '/og-pkg-std-fs.png');
		assert.equal(ogImageFor('/playground', MANIFEST), DEFAULT_OG_IMAGE);
		assert.equal(ogImageFor('/packages/@std/nope', MANIFEST), DEFAULT_OG_IMAGE);
	});
});
