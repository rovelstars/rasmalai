import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	BASE_SCHEMA,
	dayString,
	extractDepNames,
	matchesPackageFilter,
	validateKeywords,
	type PackageSummary
} from '../src/lib/server/db.js';
import { parseTarEntries, decodeFileView } from '../src/lib/server/chunks.js';
import { humanDate, shortDownloads } from '../src/lib/packages-meta.js';

function tarEntry(name: string, data: Uint8Array, typeflag = '0', prefix = ''): Uint8Array {
	const header = new Uint8Array(512);
	header.set(new TextEncoder().encode(name.slice(0, 100)));
	if (prefix) header.set(new TextEncoder().encode(prefix.slice(0, 155)), 345);
	const size = data.length.toString(8).padStart(11, '0') + ' ';
	header.set(new TextEncoder().encode(size), 124);
	header[156] = typeflag.charCodeAt(0);
	const blocks = Math.ceil(data.length / 512);
	const out = new Uint8Array(512 + blocks * 512);
	out.set(header);
	out.set(data, 512);
	return out;
}

function summary(over: Partial<PackageSummary> = {}): PackageSummary {
	return {
		name: 'demo',
		description: 'a demo package',
		author: 'anon',
		repository: '',
		license: 'MIT',
		downloads: 0,
		stars: 0,
		tags: [],
		keywords: [],
		updatedAt: 1_800_000_000,
		latest: '1.0.0',
		versionCount: 1,
		dependents: 0,
		...over
	};
}

describe('validateKeywords', () => {
	it('accepts missing and empty lists', () => {
		assert.deepEqual(validateKeywords(undefined), []);
		assert.deepEqual(validateKeywords([]), []);
	});

	it('accepts lowercase, digits and hyphens up to 12', () => {
		assert.deepEqual(validateKeywords(['http', 'web-2', 'x3']), ['http', 'web-2', 'x3']);
		assert.equal(validateKeywords(Array.from({ length: 12 }, (_, i) => `k${i}`))?.length, 12);
	});

	it('dedupes while keeping order', () => {
		assert.deepEqual(validateKeywords(['a', 'b', 'a']), ['a', 'b']);
	});

	it('rejects uppercase, spaces, empty, too many and non-arrays', () => {
		assert.equal(validateKeywords(['HTTP']), null);
		assert.equal(validateKeywords(['has space']), null);
		assert.equal(validateKeywords(['']), null);
		assert.equal(validateKeywords(['under_score']), null);
		assert.equal(validateKeywords(Array.from({ length: 13 }, (_, i) => `k${i}`)), null);
		assert.equal(validateKeywords('http'), null);
		assert.equal(validateKeywords([42]), null);
		assert.equal(validateKeywords(null), null);
	});
});

describe('extractDepNames', () => {
	it('keeps registry names, scoped and unscoped', () => {
		const names = extractDepNames(JSON.stringify({ deps: { left: '^1.0.0', '@acme/util': '~2.1.0', app: '*' } }));
		assert.deepEqual(names, ['@acme/util', 'app', 'left']);
	});

	it('skips path, git, native and url shapes', () => {
		const names = extractDepNames(
			JSON.stringify({
				deps: {
					keep: '1.2.3',
					'./local': '*',
					'/abs/path': '*',
					'file:../x': '*',
					'git+https://example.com/r.git': '*',
					'https://example.com/t.tgz': '*',
					'native:fs': '*'
				}
			})
		);
		assert.deepEqual(names, ['keep']);
	});

	it('returns [] for missing, non-object or broken manifests', () => {
		assert.deepEqual(extractDepNames('{}'), []);
		assert.deepEqual(extractDepNames(JSON.stringify({ deps: ['left'] })), []);
		assert.deepEqual(extractDepNames('not json'), []);
	});
});

describe('decodeFileView', () => {
	it('returns text for decodable files', () => {
		const r = decodeFileView(new TextEncoder().encode('print "hi"'));
		assert.equal(r.ok, true);
	});

	it('maps oversized and binary files', () => {
		assert.deepEqual(decodeFileView(new Uint8Array(256 * 1024 + 1)), { ok: false, code: 'too-large' });
		assert.deepEqual(decodeFileView(new Uint8Array([0xff, 0xfe, 0x00])), { ok: false, code: 'binary' });
	});
});

describe('dayString', () => {
	it('formats UTC days', () => {
		assert.equal(dayString(1741564800), '2025-03-10');
	});
});

describe('shortDownloads', () => {
	const vectors: Array<[number, string]> = [
		[0, '0'],
		[999, '999'],
		[1000, '1k'],
		[1200, '1.2k'],
		[15400, '15.4k'],
		[999999, '1M'],
		[1000000, '1M'],
		[3400000, '3.4M'],
		[1000000000, '1B'],
		[2500000000, '2.5B']
	];
	for (const [n, want] of vectors) {
		it(`${n} -> ${want}`, () => {
			assert.equal(shortDownloads(n), want);
		});
	}

	it('floors fractions and guards junk', () => {
		assert.equal(shortDownloads(1999.9), '2k');
		assert.equal(shortDownloads(NaN), '0');
		assert.equal(shortDownloads(-5), '0');
	});
});

describe('humanDate', () => {
	const NOW = 1_800_000_000;

	it('covers relative times', () => {
		assert.equal(humanDate(NOW - 30, NOW), 'just now');
		assert.equal(humanDate(NOW, NOW), 'just now');
		assert.equal(humanDate(NOW - 60, NOW), '1 minute ago');
		assert.equal(humanDate(NOW - 5 * 60, NOW), '5 minutes ago');
		assert.equal(humanDate(NOW - 3600, NOW), '1 hour ago');
		assert.equal(humanDate(NOW - 5 * 3600, NOW), '5 hours ago');
		assert.equal(humanDate(NOW - 86400, NOW), '1 day ago');
		assert.equal(humanDate(NOW - 5 * 86400, NOW), '5 days ago');
		assert.equal(humanDate(NOW - 29 * 86400, NOW), '29 days ago');
	});

	it('switches to ordinal dates past 30 days', () => {
		assert.equal(humanDate(NOW - 30 * 86400, NOW), '16th December 2026');
		assert.equal(humanDate(1741564800, NOW), '10th March 2025');
	});

	it('uses st, nd, rd and th correctly', () => {
		const cases: Array<[number, string]> = [
			[1, '1st January 2025'],
			[2, '2nd January 2025'],
			[3, '3rd January 2025'],
			[11, '11th January 2025'],
			[12, '12th January 2025'],
			[13, '13th January 2025'],
			[21, '21st January 2025'],
			[22, '22nd January 2025'],
			[23, '23rd January 2025'],
			[31, '31st January 2025']
		];
		for (const [day, want] of cases) {
			assert.equal(humanDate(Date.UTC(2025, 0, day) / 1000, NOW), want);
		}
	});
});
