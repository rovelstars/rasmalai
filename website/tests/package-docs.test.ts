import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { normalizeGuideEntry, parseStoredGuides, parseApiModules } from '../src/lib/server/guides.ts';

describe('normalizeGuideEntry', () => {
	it('accepts a minimal slug/title/source entry', () => {
		const r = normalizeGuideEntry({ slug: 'quickstart', title: 'Quickstart', source: '# Hi' });
		assert.equal(r.ok, true);
		if (r.ok) {
			assert.equal(r.entry.slug, 'quickstart');
			assert.equal(r.entry.description, '');
			assert.equal(r.entry.section, null);
			assert.equal(r.entry.order, null);
			assert.ok(r.entry.html.includes('Hi'));
			assert.equal(r.entry.source, '# Hi');
		}
	});

	it('keeps description, section and order', () => {
		const r = normalizeGuideEntry({
			slug: 'auth',
			title: 'Auth',
			source: 'body',
			description: 'Log in first.',
			section: 'Start',
			order: '2'
		});
		assert.equal(r.ok, true);
		if (r.ok) {
			assert.equal(r.entry.description, 'Log in first.');
			assert.equal(r.entry.section, 'Start');
			assert.equal(r.entry.order, '2');
		}
	});

	it('nulls blank section and order', () => {
		const r = normalizeGuideEntry({ slug: 'a', title: 'A', source: '', section: '  ', order: '' });
		assert.equal(r.ok, true);
		if (r.ok) {
			assert.equal(r.entry.section, null);
			assert.equal(r.entry.order, null);
		}
	});

	it('rejects missing slug/title and bad slugs', () => {
		assert.equal(normalizeGuideEntry({ title: 'A' }).ok, false);
		assert.equal(normalizeGuideEntry({ slug: 'a' }).ok, false);
		assert.equal(normalizeGuideEntry({ slug: 'Bad Slug!', title: 'A' }).ok, false);
		assert.equal(normalizeGuideEntry(null).ok, false);
		assert.equal(normalizeGuideEntry('x').ok, false);
	});
});

describe('parseStoredGuides', () => {
	it('round-trips normalized entries and drops junk', () => {
		const r = normalizeGuideEntry({ slug: 'a', title: 'A', source: 'x' });
		assert.equal(r.ok, true);
		if (!r.ok) return;
		const stored = JSON.stringify([r.entry, { nope: 1 }]);
		const back = parseStoredGuides(stored);
		assert.equal(back.length, 1);
		assert.equal(back[0].slug, 'a');
		assert.equal(parseStoredGuides('not json').length, 0);
		assert.equal(parseStoredGuides('{}').length, 0);
	});
});

describe('parseApiModules', () => {
	it('extracts module names and tolerates junk', () => {
		assert.deepEqual(parseApiModules('{"modules":[]}'), []);
		assert.deepEqual(parseApiModules('{"modules":[{"name":"net"}]}'), [{ name: 'net' }]);
		assert.equal(parseApiModules('{"modules":[{"name":7}]}').length, 0);
		assert.equal(parseApiModules('broken').length, 0);
		assert.equal(parseApiModules('[]').length, 0);
	});
});
