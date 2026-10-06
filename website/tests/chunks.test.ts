import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { chunkTarball, rebuildTarball, sha256Hex } from '../src/lib/server/chunks.js';

function tarEntry(name: string, data: Uint8Array): Uint8Array {
	const header = new Uint8Array(512);
	header.set(new TextEncoder().encode(name.slice(0, 100)));
	const oct = (off: number, len: number, value: number) => {
		const digits = value.toString(8);
		const pad = len - 1 - digits.length;
		for (let i = 0; i < pad; i++) header[off + i] = 48;
		for (let i = 0; i < digits.length; i++) header[off + pad + i] = digits.charCodeAt(i);
		header[off + len - 1] = 0;
	};
	oct(100, 8, 0o644);
	oct(108, 8, 0);
	oct(116, 8, 0);
	oct(124, 12, data.length);
	oct(136, 12, 0);
	for (let i = 148; i < 156; i++) header[i] = 32;
	header[156] = 48;
	header.set(new TextEncoder().encode('ustar\0'), 257);
	header.set(new TextEncoder().encode('00'), 263);
	header.set(new TextEncoder().encode('rnx'), 265);
	header.set(new TextEncoder().encode('rnx'), 297);
	let sum = 0;
	for (let i = 0; i < 512; i++) sum += header[i];
	const sumText = sum.toString(8).padStart(6, '0');
	for (let i = 0; i < 6; i++) header[148 + i] = sumText.charCodeAt(i);
	header[154] = 0;
	header[155] = 32;
	const blocks = Math.ceil(data.length / 512);
	const out = new Uint8Array(512 + blocks * 512);
	out.set(header);
	out.set(data, 512);
	return out;
}

const concat = (xs: Uint8Array[]): Uint8Array => {
	const out = new Uint8Array(xs.reduce((n, x) => n + x.length, 0));
	let off = 0;
	for (const x of xs) {
		out.set(x, off);
		off += x.length;
	}
	return out;
};

describe('chunk store', () => {
	it('dedups identical files while keeping both entries', async () => {
		const a = new TextEncoder().encode('hello world');
		const tar = concat([tarEntry('a.txt', a), tarEntry('b.txt', a), new Uint8Array(1024)]);
		const { units, entries } = await chunkTarball(tar);
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

	it('round-trips tar bytes through chunks byte-identically', async () => {
		const a = new TextEncoder().encode('hello world');
		const big = new Uint8Array(200 * 1024);
		for (let off = 0; off < big.length; off += 65536) {
			crypto.getRandomValues(big.subarray(off, Math.min(off + 65536, big.length)));
		}
		const tar = concat([tarEntry('a.txt', a), tarEntry('big.bin', big), new Uint8Array(1024)]);
		const { units, entries } = await chunkTarball(tar);
		const byHash = new Map(units.map((u) => [u.hash, u.bytes] as [string, Uint8Array]));
		const rebuilt = rebuildTarball(entries, byHash);
		assert.deepEqual(rebuilt, tar);
	});

	it('cdc-splits large files and keeps small ones whole', async () => {
		const big = new Uint8Array(200 * 1024);
		for (let off = 0; off < big.length; off += 65536) {
			crypto.getRandomValues(big.subarray(off, Math.min(off + 65536, big.length)));
		}
		const tar = concat([tarEntry('big.bin', big), new Uint8Array(1024)]);
		const { units } = await chunkTarball(tar);
		assert.ok(units.length > 1);
		const total = units.reduce((n, u) => n + u.size, 0);
		assert.equal(total, big.length);
	});

	it('pins the tar header layout both implementations share', async () => {
		const hello = new TextEncoder().encode('hello');
		const h = 'H'.repeat(64);
		const out = rebuildTarball([{ name: 'a.txt', size: 5, dir: false, chunks: [h] }], new Map([[h, hello]]));
		assert.equal(out.length, 2048);
		assert.equal(
			Buffer.from(out.subarray(0, 64)).toString('hex'),
			'612e7478740000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000'
		);
		assert.equal(
			Buffer.from(out.subarray(256, 320)).toString('hex'),
			'007573746172003030726e780000000000000000000000000000000000000000000000000000000000726e780000000000000000000000000000000000000000'
		);
		assert.deepEqual(out.subarray(512, 517), hello);
	});

	it('round-trips opaque blobs without archive markers', async () => {
		const gz = new Uint8Array([0x1f, 0x8b, 0x08, 1, 2, 3, 4, 5]);
		const { units, entries } = await chunkTarball(gz);
		assert.equal(entries.length, 1);
		assert.equal(entries[0].name, '');
		const byHash = new Map(units.map((u) => [u.hash, u.bytes] as [string, Uint8Array]));
		const rebuilt = rebuildTarball(entries, byHash);
		assert.deepEqual(rebuilt, gz);
		assert.equal(await sha256Hex(rebuilt), await sha256Hex(gz));
	});
});
