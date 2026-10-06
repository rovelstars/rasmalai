import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	CAPABILITY_DOMAINS,
	parseManifestPermissions,
	splitPermission
} from '../src/lib/server/permissions.js';

describe('splitPermission', () => {
	it('splits parametric capabilities into domain and opaque arg', () => {
		assert.deepEqual(splitPermission('fs:read:/data'), { domain: 'fs:read', arg: '/data' });
		assert.deepEqual(splitPermission('net:http:example.com'), {
			domain: 'net:http',
			arg: 'example.com'
		});
		assert.deepEqual(splitPermission('sys:exec:ffmpeg'), { domain: 'sys:exec', arg: 'ffmpeg' });
		assert.deepEqual(splitPermission('env:read:HOME'), { domain: 'env:read', arg: 'HOME' });
		assert.deepEqual(splitPermission('env:read:*'), { domain: 'env:read', arg: '*' });
	});

	it('keeps colons inside the arg opaque', () => {
		assert.deepEqual(splitPermission('fs:read:a:b'), { domain: 'fs:read', arg: 'a:b' });
	});

	it('treats bare capabilities as arg-less domains', () => {
		for (const bare of [
			'fs:delegated',
			'net:delegated',
			'unsafe:ffi',
			'unsafe:raw_memory',
			'term:write',
			'term:read',
			'term:raw',
			'env:dump'
		]) {
			assert.deepEqual(splitPermission(bare), { domain: bare, arg: '' });
		}
	});

	it('rejects malformed and unknown permissions', () => {
		for (const bad of ['', 'bogus', 'bogus:cap', 'fs:read', 'native:zlib', 'term:write:x', 'fs:run:/x']) {
			assert.throws(() => splitPermission(bad), /malformed|unknown/);
		}
	});

	it('covers every domain head it can return', () => {
		const domains: Set<string> = new Set(CAPABILITY_DOMAINS);
		assert.equal(domains.size, CAPABILITY_DOMAINS.length);
		for (const d of ['fs:read', 'net:http', 'sys:exec', 'env:read', 'unsafe:ffi', 'term:write']) {
			assert.ok(domains.has(d), d);
		}
	});
});

describe('parseManifestPermissions', () => {
	it('returns [] when the manifest declares nothing', () => {
		assert.deepEqual(parseManifestPermissions('{}'), []);
		assert.deepEqual(parseManifestPermissions('{"deps":{}}'), []);
		assert.deepEqual(parseManifestPermissions('not json'), []);
	});

	it('parses string and table entries', () => {
		const out = parseManifestPermissions(
			JSON.stringify({
				permissions: ['term:write', { perm: 'fs:read:/data', reason: 'seed fixtures' }]
			})
		);
		assert.deepEqual(out, [
			{ domain: 'term:write', arg: '', reason: null },
			{ domain: 'fs:read', arg: '/data', reason: 'seed fixtures' }
		]);
	});

	it('treats a missing reason as null', () => {
		const out = parseManifestPermissions(JSON.stringify({ permissions: [{ perm: 'fs:delegated' }] }));
		assert.deepEqual(out, [{ domain: 'fs:delegated', arg: '', reason: null }]);
	});

	it('rejects malformed manifests', () => {
		assert.throws(() => parseManifestPermissions('{"permissions":"x"}'), /array/);
		assert.throws(() => parseManifestPermissions('{"permissions":["bogus:cap"]}'), /unknown|malformed/);
		assert.throws(
			() => parseManifestPermissions('{"permissions":[{ "perm": "term:write", "reason": 7 }]}'),
			/reason/
		);
		assert.throws(() => parseManifestPermissions('{"permissions":[{ "reason": "x" }]}'), /perm/);
		assert.throws(() => parseManifestPermissions('{"permissions":[42]}'), /strings/);
	});
});
