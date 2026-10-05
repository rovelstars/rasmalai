import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	BASE_SCHEMA,
	MAX_FILE_BYTES,
	dayString,
	extractDepNames,
	matchesPackageFilter,
	readFileIndex,
	selectVersionFile,
	validateKeywords,
	type PackageSummary
} from '../src/lib/server/db.js';
import { parseTarEntries } from '../src/lib/server/chunks.js';
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

describe('readFileIndex', () => {
	it('reads path/size pairs', () => {
		const files = readFileIndex(JSON.stringify({ files: [{ path: 'src/main.rnx', size: 12 }] }));
		assert.deepEqual(files, [{ path: 'src/main.rnx', size: 12 }]);
	});

	it('returns null when the files key is absent', () => {
		assert.equal(readFileIndex('{}'), null);
		assert.equal(readFileIndex(JSON.stringify({ files: 'nope' })), null);
		assert.equal(readFileIndex('broken'), null);
	});

	it('skips malformed entries but keeps the index', () => {
		const files = readFileIndex(JSON.stringify({ files: [{ path: '', size: 1 }, { size: 2 }, { path: 'a', size: -1 }, { path: 'b', size: 3 }] }));
		assert.deepEqual(files, [{ path: 'b', size: 3 }]);
	});
});

describe('matchesPackageFilter', () => {
	const NOW = 1_800_000_000;
	const base = summary({
		name: '@acme/http-server',
		description: 'Tiny HTTP server',
		keywords: ['http', 'server'],
		license: 'MIT',
		downloads: 1500,
		updatedAt: NOW - 2 * 86400
	});

	it('matches substrings on name and description', () => {
		assert.equal(matchesPackageFilter(base, { search: 'http' }, NOW), true);
		assert.equal(matchesPackageFilter(base, { search: 'tiny' }, NOW), true);
		assert.equal(matchesPackageFilter(base, { search: 'missing' }, NOW), false);
	});

	it('combines every filter', () => {
		const f = { search: 'http', keyword: 'server', license: 'MIT', minDownloads: 1000, updatedSinceDays: 7 };
		assert.equal(matchesPackageFilter(base, f, NOW), true);
		assert.equal(matchesPackageFilter(base, { ...f, keyword: 'tls' }, NOW), false);
		assert.equal(matchesPackageFilter(base, { ...f, license: 'Apache-2.0' }, NOW), false);
		assert.equal(matchesPackageFilter(base, { ...f, minDownloads: 99999 }, NOW), false);
		assert.equal(matchesPackageFilter(base, { ...f, updatedSinceDays: 1 }, NOW), false);
		assert.equal(matchesPackageFilter(base, {}, NOW), true);
	});
});

describe('registry SQL shapes', () => {
	it('declares keywords, package_deps and download_daily', () => {
		assert.match(BASE_SCHEMA, /keywords TEXT DEFAULT ''/);
		assert.match(
			BASE_SCHEMA,
			/CREATE TABLE IF NOT EXISTS package_deps \(\s*package_id TEXT NOT NULL REFERENCES packages\(id\),\s*dep_name TEXT NOT NULL,\s*PRIMARY KEY \(package_id, dep_name\)\s*\)/
		);
		assert.match(BASE_SCHEMA, /CREATE TABLE IF NOT EXISTS download_daily \(/);
		assert.match(BASE_SCHEMA, /CREATE INDEX IF NOT EXISTS idx_deps_name ON package_deps\(dep_name\)/);
	});
});

describe('parseTarEntries', () => {
	it('round-trips named entries', () => {
		const a = new TextEncoder().encode('hello');
		const b = new TextEncoder().encode('world!');
		const tar = new Uint8Array([
			...tarEntry('src/main.rnx', a),
			...tarEntry('README.md', b),
			...new Uint8Array(1024)
		]);
		const entries = parseTarEntries(tar);
		assert.deepEqual(
			entries.map((e) => e.name),
			['src/main.rnx', 'README.md']
		);
		assert.deepEqual(entries[0].bytes, a);
		assert.deepEqual(entries[1].bytes, b);
	});

	it('skips directories and joins ustar prefixes', () => {
		const tar = new Uint8Array([
			...tarEntry('src', new Uint8Array(0), '5'),
			...tarEntry('main.rnx', new TextEncoder().encode('x'), '0', 'src'),
			...new Uint8Array(1024)
		]);
		const entries = parseTarEntries(tar);
		assert.deepEqual(
			entries.map((e) => e.name),
			['src/main.rnx']
		);
	});
});

describe('selectVersionFile', () => {
	const entries = [
		{ name: 'src/main.rnx', bytes: new TextEncoder().encode('print "hi"') },
		{ name: 'bin/blob', bytes: new Uint8Array([0xff, 0xfe, 0x00]) },
		{ name: 'big.txt', bytes: new Uint8Array(MAX_FILE_BYTES + 1) }
	];

	it('returns text for decodable files', () => {
		const r = selectVersionFile(entries, 'src/main.rnx');
		assert.equal(r.ok, true);
		assert.equal(r.ok && r.path, 'src/main.rnx');
		assert.equal(r.ok && r.text, 'print "hi"');
		assert.equal(r.ok && r.size, 10);
	});

	it('maps unknown paths to 404', () => {
		const r = selectVersionFile(entries, 'nope.txt');
		assert.deepEqual(r, { ok: false, code: 'not-found', status: 404 });
	});

	it('maps oversized and binary files to 415', () => {
		assert.deepEqual(selectVersionFile(entries, 'big.txt'), { ok: false, code: 'too-large', status: 415 });
		assert.deepEqual(selectVersionFile(entries, 'bin/blob'), { ok: false, code: 'binary', status: 415 });
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
