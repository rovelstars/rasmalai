import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { buildPackagePageNav } from '../src/lib/docs/package-pages.ts';

describe('buildPackagePageNav', () => {
	it('keeps single-page sections in the unheaded list', () => {
		const nav = buildPackagePageNav([
			{ slug: 'guides/floats', title: 'Float vectors', section: 'Float vectors', order: '20' },
			{ slug: 'guides/ints', title: 'Integer vectors', section: 'Integer vectors', order: '30' }
		]);
		assert.deepEqual(
			nav.loose.map((p) => p.slug),
			['guides/floats', 'guides/ints']
		);
		assert.deepEqual(nav.sections, []);
	});

	it('keeps multi-page sections as named groups', () => {
		const nav = buildPackagePageNav([
			{ slug: 'guides/urls', title: 'URLs', section: 'URLs', order: '20' },
			{ slug: 'guides/query', title: 'Query strings', section: 'URLs', order: '30' },
			{ slug: 'guides/headers', title: 'Headers', section: 'HTTP', order: '40' },
			{ slug: 'guides/requests', title: 'Requests', section: 'HTTP', order: '50' }
		]);
		assert.deepEqual(
			nav.sections.map((s) => s.section),
			['URLs', 'HTTP']
		);
		assert.deepEqual(
			nav.sections[0].pages.map((p) => p.slug),
			['guides/urls', 'guides/query']
		);
	});

	it('orders pages and builds a flat reading order', () => {
		const nav = buildPackagePageNav([
			{ slug: 'guides/late', title: 'Late', order: '30' },
			{ slug: 'overview', title: 'Ignored', order: '0' },
			{ slug: 'guides/early', title: 'Early', order: '20' },
			{ slug: 'api', title: 'Ignored API', order: '99' }
		]);
		assert.deepEqual(
			nav.order.map((p) => p.slug),
			['overview', 'getting-started', 'guides/early', 'guides/late', 'api']
		);
	});

	it('sorts named sections by their first page order', () => {
		const nav = buildPackagePageNav([
			{ slug: 'guides/b1', title: 'B1', section: 'B', order: '50' },
			{ slug: 'guides/b2', title: 'B2', section: 'B', order: '60' },
			{ slug: 'guides/a1', title: 'A1', section: 'A', order: '20' },
			{ slug: 'guides/a2', title: 'A2', section: 'A', order: '30' }
		]);
		assert.deepEqual(
			nav.sections.map((s) => s.section),
			['A', 'B']
		);
	});
});
