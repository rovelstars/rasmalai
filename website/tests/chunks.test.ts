import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { chunkTarball, splitTarFiles, sha256Hex } from '../src/lib/server/chunks.js';

function tarEntry(name: string, data: Uint8Array): Uint8Array {
	const header = new Uint8Array(512);
	header.set(new TextEncoder().encode(name.slice(0, 100)));
	const size = data.length.toString(8).padStart(11, '0') + ' ';
	header.set(new TextEncoder().encode(size), 124);
	header[156] = 48;
	const blocks = Math.ceil(data.length / 512);
	const out = new Uint8Array(512 + blocks * 512);
	out.set(header);
	out.set(data, 512);
	return out;
}

describe('chunk store', () => {
	it('keeps ordered units with repeats for identical files', async () => {
		const a = new TextEncoder().encode('hello world');
		const tar = new Uint8Array([
			...tarEntry('a.txt', a),
			...tarEntry('b.txt', a),
			...new Uint8Array(1024)
		]);
		assert.equal(splitTarFiles(tar).length, 2);
		const units = await chunkTarball(tar);
		assert.equal(units.length, 2);
		assert.equal(units[0].hash, await sha256Hex(a));
		assert.equal(units[1].hash, units[0].hash);
	});

	it('round-trips tar files through chunks to bytes', async () => {
		const a = new TextEncoder().encode('hello world');
		const big = new Uint8Array(200 * 1024);
		for (let off = 0; off < big.length; off += 65536) {
			crypto.getRandomValues(big.subarray(off, Math.min(off + 65536, big.length)));
		}
		const tar = new Uint8Array([...tarEntry('a.txt', a), ...tarEntry('big.bin', big), ...new Uint8Array(1024)]);
		const files = splitTarFiles(tar);
		const units = await chunkTarball(tar);
		const joined = Buffer.concat(units.map((u) => Buffer.from(u.bytes)));
		assert.equal(joined.length, files.reduce((n, f) => n + f.length, 0));
		assert.deepEqual(joined, Buffer.concat(files.map((f) => Buffer.from(f))));
	});

	it('cdc-splits large files and keeps small ones whole', async () => {
		const big = new Uint8Array(200 * 1024);
		for (let off = 0; off < big.length; off += 65536) {
			crypto.getRandomValues(big.subarray(off, Math.min(off + 65536, big.length)));
		}
		const tar = new Uint8Array([...tarEntry('big.bin', big), ...new Uint8Array(1024)]);
		const units = await chunkTarball(tar);
		assert.ok(units.length > 1);
		const total = units.reduce((n, u) => n + u.size, 0);
		assert.equal(total, big.length);
	});
});
