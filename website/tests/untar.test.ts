import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { untar, gunzip, isGzip } from '../src/lib/docs/untar.ts';

function tarEntry(name: string, data: Uint8Array): Uint8Array {
	const header = new Uint8Array(512);
	const enc = new TextEncoder();
	header.set(enc.encode(name).subarray(0, 100));
	const oct = (off: number, len: number, value: number) => {
		const digits = value.toString(8);
		for (let i = 0; i < len - digits.length - 1; i++) header[off + i] = 48;
		for (let i = 0; i < digits.length; i++) header[off + len - digits.length - 1 + i] = digits.charCodeAt(i);
	};
	oct(100, 8, 0o644);
	oct(124, 12, data.length);
	oct(136, 12, 0);
	header[156] = 48;
	header.set(enc.encode('ustar'), 257);
	let sum = 0;
	for (let i = 148; i < 156; i++) header[i] = 32;
	for (let i = 0; i < 512; i++) sum += header[i];
	const sumText = sum.toString(8).padStart(6, '0');
	for (let i = 0; i < 6; i++) header[148 + i] = sumText.charCodeAt(i);
	const blocks = Math.ceil(data.length / 512);
	const out = new Uint8Array(512 + blocks * 512 + 1024);
	out.set(header);
	out.set(data, 512);
	return out;
}

describe('untar', () => {
	it('extracts files and skips dirs and padding', () => {
		const a = new TextEncoder().encode('hello');
		const tar = new Uint8Array([...tarEntry('a.txt', a), ...new Uint8Array(512)]);
		const files = untar(tar);
		assert.equal(files.length, 1);
		assert.equal(files[0].path, 'a.txt');
		assert.deepEqual(files[0].bytes, a);
	});

	it('rejects DotDot paths', () => {
		const tar = tarEntry('../evil.txt', new TextEncoder().encode('x'));
		assert.equal(untar(tar).length, 0);
	});

	it('round-trips gzip through gunzip', async () => {
		const raw = new TextEncoder().encode('compress me please compress me please');
		const gz = new Uint8Array(gzipSync(raw));
		assert.equal(isGzip(gz), true);
		assert.equal(isGzip(raw), false);
		assert.deepEqual(await gunzip(gz), raw);
	});

	it('unpacks a gzipped tar end to end', async () => {
		const a = new TextEncoder().encode('export fn hello(): Int { return 1; }');
		const tar = new Uint8Array([...tarEntry('src/main.rnx', a), ...new Uint8Array(1024)]);
		const files = untar(await gunzip(new Uint8Array(gzipSync(tar))));
		assert.equal(files.length, 1);
		assert.equal(files[0].path, 'src/main.rnx');
		assert.deepEqual(files[0].bytes, a);
	});
});
