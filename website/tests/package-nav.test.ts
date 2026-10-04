import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { buildPackageNav } from '../src/lib/docs/package-nav.ts';

describe('buildPackageNav', () => {
	it('groups by section', () => {
		const groups = buildPackageNav(
			[
				{ slug: 'a', title: 'A', description: 'd', section: 'Start' },
				{ slug: 'b', title: 'B', description: 'd', section: 'Advanced' },
				{ slug: 'c', title: 'C', description: 'd', section: 'Start' }
			],
			[]
		);
		const sections = groups.map((g) => g.section);
		assert.ok(sections.includes('Start'));
		assert.ok(sections.includes('Advanced'));
		const start = groups.find((g) => g.section === 'Start');
		assert.deepEqual(
			start?.chapters.map((c) => c.slug),
			['a', 'c']
		);
	});

	it('honors explicit order over slugs', () => {
		const groups = buildPackageNav(
			[
				{ slug: '01-a', title: 'A', description: 'd', section: 'S', order: '2' },
				{ slug: '02-b', title: 'B', description: 'd', section: 'S', order: '1' }
			],
			[]
		);
		assert.equal(groups.length, 2);
		assert.deepEqual(
			groups[0].chapters.map((c) => c.slug),
			['02-b', '01-a']
		);
	});

	it('places ungrouped pages last', () => {
		const groups = buildPackageNav(
			[
				{ slug: 'solo', title: 'Solo', description: 'd' },
				{ slug: 'g1', title: 'G1', description: 'd', section: 'Guides', order: '1' }
			],
			[]
		);
		const body = groups.filter((g) => g.section !== 'API Reference');
		assert.equal(body[body.length - 1].section, '');
		assert.deepEqual(body[body.length - 1].chapters.map((c) => c.slug), ['solo']);
	});

	it('appends the API-reference entry exactly once with all module names', () => {
		const groups = buildPackageNav(
			[{ slug: 'intro', title: 'Intro', description: 'd', section: 'Start' }],
			['store', 'net']
		);
		const api = groups.filter((g) => g.section === 'API Reference');
		assert.equal(api.length, 1);
		assert.equal(groups[groups.length - 1].section, 'API Reference');
		assert.deepEqual(
			api[0].chapters.map((c) => c.slug).sort(),
			['net', 'store']
		);
	});

	it('ignores the icon key even if present in input', () => {
		const groups = buildPackageNav(
			[
				{
					slug: 'x',
					title: 'X',
					description: 'd',
					section: 'S',
					icon: 'Rocket'
				} as unknown as { slug: string; title: string; description: string; section: string }
			],
			[]
		);
		for (const g of groups) {
			for (const c of g.chapters) {
				assert.equal(c.icon, null);
			}
		}
	});
});
