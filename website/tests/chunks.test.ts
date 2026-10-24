import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	chunkCanonicalFiles,
	chunkCountBound,
	manifestDigest,
	manifestDigestBytes,
	sha256Hex,
	sha256HexConcat,
	checkManifestEntries,
	UnsafeEntryNameError
} from '../src/lib/server/chunks.js';

const enc = new TextEncoder();

describe('canonical chunk store', () => {
	it('dedups identical files while keeping both entries', async () => {
		const a = enc.encode('hello world');
		const { units, entries } = await chunkCanonicalFiles([
			{ name: 'a.txt', bytes: a },
			{ name: 'b.txt', bytes: a }
		]);
		assert.equal(units.length, 1);
		assert.equal(units[0].hash, await sha256Hex(a));
		assert.deepEqual(
			entries.map((e) => [e.name, e.size]),
			[
				['a.txt', a.length],
				['b.txt', a.length]
			]
		);
	});

	it('cdc-splits large files and keeps small ones whole', async () => {
		const big = new Uint8Array(200 * 1024);
		for (let off = 0; off < big.length; off += 65536) {
			crypto.getRandomValues(big.subarray(off, Math.min(off + 65536, big.length)));
		}
		const { units, entries } = await chunkCanonicalFiles([{ name: 'big.bin', bytes: big }]);
		assert.ok(units.length > 1);
		const total = units.reduce((n, u) => n + u.size, 0);
		assert.equal(total, big.length);
		assert.equal(entries.length, 1);
		assert.equal(entries[0].size, big.length);
		const reassembled = Buffer.concat(units.map((u) => Buffer.from(u.bytes)));
		assert.deepEqual(new Uint8Array(reassembled), big);
	});

	it('refuses unsafe entry names', async () => {
		await assert.rejects(
			() => chunkCanonicalFiles([{ name: '../escape.txt', bytes: enc.encode('x') }]),
			UnsafeEntryNameError
		);
		assert.ok(checkManifestEntries([{ name: 'ok.txt', size: 1, dir: false, chunks: [] }]) === null);
	});

	it('counts the chunks of a package before any byte is hashed', () => {
		assert.equal(chunkCountBound([{ name: 'a.txt', bytes: new Uint8Array(10) }]), 1);
		const big = { name: 'big.bin', bytes: new Uint8Array(200 * 1024) };
		assert.ok(chunkCountBound([big]) > 1);
	});
});

describe('manifest digest', () => {
	it('encodes the canonical entry bytes deterministically', async () => {
		const entries = [
			{ name: 'a.txt', size: 5, dir: false, chunks: ['00'.repeat(32)] },
			{ name: 'b/', size: 0, dir: true, chunks: [] as string[] }
		];
		const a = manifestDigestBytes(entries);
		const b = manifestDigestBytes(entries);
		assert.deepEqual(a, b);
		const digest = await manifestDigest(entries);
		assert.match(digest, /^[0-9a-f]{64}$/);
		assert.equal(digest, await sha256Hex(a));
		const swapped = manifestDigestBytes([entries[1], entries[0]]);
		assert.notDeepEqual(a, swapped);
	});

	it('pins the canonical digest encoding against the Rust client', async () => {
		const entries = [
			{ name: 'Project.config', size: 7, dir: false, chunks: ['ab'.repeat(32)] },
			{ name: 'src/main.rnx', size: 3, dir: false, chunks: ['cd'.repeat(32), 'ef'.repeat(32)] }
		];
		const bytes = manifestDigestBytes(entries);
		assert.equal(
			Buffer.from(bytes.subarray(0, 32)).toString('hex'),
			'0e00000050726f6a6563742e636f6e66696707000000000000000001000000ab'
		);
		const digest = await manifestDigest(entries);
		assert.equal(digest, 'ca56370c89bc36239f29b98d936594fa3ddb6558ae693343511f3bfe8025b1e6');
	});
});

describe('batched hashing', () => {
	it('hashes a byte stream in one call', async () => {
		const a = enc.encode('hello ');
		const b = enc.encode('world');
		assert.equal(await sha256HexConcat([a, b]), await sha256Hex(enc.encode('hello world')));
	});
});
