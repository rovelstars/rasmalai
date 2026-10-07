import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	concatChunks,
	entryCandidates,
	fetchStdModule,
	findFileEntry,
	isDevOrigin,
	resolveRegistryOrigin,
	sha256Hex,
	stdPackageOf,
	type ChunkEntry
} from '../src/lib/playground/stdvfs.ts';
import { createHash } from 'node:crypto';

describe('stdvfs package mapping', () => {
	it('maps a rest to its top-level package and entry candidates', () => {
		assert.equal(stdPackageOf('fs'), '@std/fs');
		assert.equal(stdPackageOf('net/http'), '@std/net');
		assert.deepEqual(entryCandidates('fs'), ['src/lib.rnx']);
		assert.deepEqual(entryCandidates('net/http'), ['src/http.rnx', 'src/http/mod.rnx', 'src/http/index.rnx']);
	});

	it('finds the first matching file entry', () => {
		const entries: ChunkEntry[] = [
			{ name: 'Project.config', size: 10, dir: false, chunks: ['a'.repeat(64)] },
			{ name: 'src/http.rnx', size: 3, dir: false, chunks: ['b'.repeat(64)] }
		];
		assert.equal(findFileEntry(entries, entryCandidates('net/http'))?.name, 'src/http.rnx');
		assert.equal(findFileEntry(entries, entryCandidates('fs')), null);
		assert.equal(findFileEntry(entries, ['src/http/mod.rnx']), null);
	});

	it('concatenates chunks in order', () => {
		const out = concatChunks([new Uint8Array([1, 2]), new Uint8Array([3])]);
		assert.deepEqual([...out], [1, 2, 3]);
		assert.equal(concatChunks([]).length, 0);
	});

	it('sha256 matches node crypto', async () => {
		const bytes = new TextEncoder().encode('export fn hello(): Int { return 1; }\n');
		const want = createHash('sha256').update(bytes).digest('hex');
		assert.equal(await sha256Hex(bytes), want);
	});
});

function mockFetch(files: Map<string, Uint8Array>, chunks: Map<string, Uint8Array>) {
	return async (url: string) => {
		const u = new URL(url);
		if (u.pathname.endsWith('/chunks')) {
			const body = JSON.stringify({
				entries: [...files.entries()].map(([name, bytes]) => ({
					name,
					size: bytes.length,
					dir: false,
					chunks: [createHash('sha256').update(bytes).digest('hex')]
				}))
			});
			return new Response(body, { status: 200 });
		}
		const m = u.pathname.match(/\/chunk\/([0-9a-f]{64})$/);
		if (m && chunks.has(m[1]!)) {
			return new Response(chunks.get(m[1]!) as BodyInit, { status: 200 });
		}
		return new Response('nope', { status: 404 });
	};
}

describe('fetchStdModule', () => {
	it('reassembles a module file from its chunks', async () => {
		const src = new TextEncoder().encode('export fn mkdir(): Int { return 0; }\n');
		const hash = createHash('sha256').update(src).digest('hex');
		const fetchImpl = mockFetch(
			new Map([['src/lib.rnx', src]]),
			new Map([[hash, src]])
		) as typeof fetch;
		const text = await fetchStdModule('fs', '0.1.0', 'https://example.test', fetchImpl);
		assert.equal(text, new TextDecoder().decode(src));
	});

	it('resolves submodule files inside the top-level package', async () => {
		const src = new TextEncoder().encode('export fn get(): Int { return 0; }\n');
		const hash = createHash('sha256').update(src).digest('hex');
		const fetchImpl = mockFetch(
			new Map([
				['src/lib.rnx', new TextEncoder().encode('net root\n')],
				['src/http.rnx', src]
			]),
			new Map([[hash, src]])
		) as typeof fetch;
		const text = await fetchStdModule('net/http', '0.1.0', 'https://example.test', fetchImpl);
		assert.ok(text.includes('export fn get'));
	});

	it('fails closed on a missing file entry', async () => {
		const fetchImpl = mockFetch(new Map(), new Map()) as typeof fetch;
		await assert.rejects(fetchStdModule('fs', '0.1.0', 'https://example.test', fetchImpl), /ships no src\/lib\.rnx/);
	});

	it('fails closed on a chunk integrity mismatch', async () => {
		const src = new TextEncoder().encode('tampered bytes\n');
		const real = new TextEncoder().encode('real bytes\n');
		const hash = createHash('sha256').update(real).digest('hex');
		const fetchImpl = (async (url: string) => {
			if (url.split('?')[0]!.endsWith('/chunks')) {
				return new Response(
					JSON.stringify({ entries: [{ name: 'src/lib.rnx', size: src.length, dir: false, chunks: [hash] }] }),
					{ status: 200 }
				);
			}
			return new Response(src as BodyInit, { status: 200 });
		}) as typeof fetch;
		await assert.rejects(fetchStdModule('fs', '0.1.0', 'https://example.test', fetchImpl), /integrity/);
	});

	it('fetches under the origin-direct namespace past stale edge generations', async () => {
		const src = new TextEncoder().encode('export fn mkdir(): Int { return 0; }\n');
		const hash = createHash('sha256').update(src).digest('hex');
		const seen: string[] = [];
		const inner = mockFetch(new Map([['src/lib.rnx', src]]), new Map([[hash, src]]));
		const fetchImpl = (async (url: string) => {
			seen.push(url);
			return inner(url);
		}) as typeof fetch;
		const text = await fetchStdModule('fs', '0.1.0', 'https://example.test', fetchImpl);
		assert.equal(text, new TextDecoder().decode(src));
		assert.ok(seen.length > 0);
		assert.ok(seen.every((u) => u.includes('origin-direct=1')));
	});

	it('fails closed on a chunks-manifest HTTP error', async () => {
		const fetchImpl = (async () => new Response('gone', { status: 410 })) as typeof fetch;
		await assert.rejects(fetchStdModule('fs', '0.1.0', 'https://example.test', fetchImpl), /HTTP 410/);
	});

	it('falls back to the fallback origin on a 404', async () => {
		const src = new TextEncoder().encode('export fn mkdir(): Int { return 0; }\n');
		const hash = createHash('sha256').update(src).digest('hex');
		const fetchImpl = (async (url: string) => {
			const u = new URL(url);
			if (u.origin === 'https://fallback.test') return mockFetch(new Map([['src/lib.rnx', src]]), new Map([[hash, src]]))(url);
			return new Response('nope', { status: 404 });
		}) as typeof fetch;
		const text = await fetchStdModule('fs', '0.1.0', 'http://localhost:5173', fetchImpl, 'https://fallback.test');
		assert.equal(text, new TextDecoder().decode(src));
	});

	it('does not fall back on non-404 errors', async () => {
		let calls = 0;
		const fetchImpl = (async () => {
			calls++;
			return new Response('gone', { status: 410 });
		}) as typeof fetch;
		await assert.rejects(
			fetchStdModule('fs', '0.1.0', 'http://localhost:5173', fetchImpl, 'https://fallback.test'),
			/HTTP 410/
		);
		assert.equal(calls, 1);
	});
});

describe('resolveRegistryOrigin', () => {
	it('uses the page origin with the prod fallback on localhost', () => {
		assert.deepEqual(resolveRegistryOrigin('http://localhost:5173'), {
			origin: 'http://localhost:5173',
			fallback: 'https://rasmalai.rovelstars.com'
		});
	});

	it('uses the page origin with no fallback in production', () => {
		assert.deepEqual(resolveRegistryOrigin('https://rasmalai.rovelstars.com'), {
			origin: 'https://rasmalai.rovelstars.com',
			fallback: undefined
		});
	});

	it('prefers the VITE_RNX_REGISTRY override with no fallback', () => {
		assert.deepEqual(resolveRegistryOrigin('http://localhost:5173', { VITE_RNX_REGISTRY: 'https://preview.test/' }), {
			origin: 'https://preview.test',
			fallback: undefined
		});
		assert.deepEqual(resolveRegistryOrigin('https://rasmalai.rovelstars.com', { VITE_RNX_REGISTRY: 'https://preview.test' }), {
			origin: 'https://preview.test',
			fallback: undefined
		});
		assert.deepEqual(resolveRegistryOrigin('http://localhost:5173', { VITE_RNX_REGISTRY: 'not a url' }), {
			origin: 'http://localhost:5173',
			fallback: 'https://rasmalai.rovelstars.com'
		});
	});

	it('rejects non-http origins', () => {
		assert.throws(() => resolveRegistryOrigin(''), /same-domain registry/);
	});

	it('detects dev origins', () => {
		assert.equal(isDevOrigin('http://localhost:5173'), true);
		assert.equal(isDevOrigin('http://127.0.0.1:4173'), true);
		assert.equal(isDevOrigin('http://thing.local:5173'), true);
		assert.equal(isDevOrigin('https://rasmalai.rovelstars.com'), false);
		assert.equal(isDevOrigin('not a url'), false);
	});
});
