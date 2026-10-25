import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import {
	feedCanonicalBytes,
	fileAdvisory,
	keyIdForRawPublicKey,
	listActiveAdvisories,
	operatorKey,
	resetOperatorKeyCache,
	signFeed,
	validateAdvisoryInput,
	verifyFeedSignature,
	withdrawAdvisory,
	type SignedFeed
} from '../src/lib/server/advisories.js';
import { GET, POST } from '../src/routes/api/v0/advisories/+server.ts';
import { POST as withdrawPOST } from '../src/routes/api/v0/advisories/[id]/withdraw/+server.ts';

let fake: FakeLibsql;
const SEED = randomBytes(32).toString('hex');
const PIN = `sha256:${'ab'.repeat(32)}`;
let env: Record<string, string | undefined>;

function filedInput(id: string, overrides: Record<string, unknown> = {}) {
	return {
		id,
		package: '@acme/widget',
		affected: '^1.0.0',
		severity: 'warning',
		title: 'test advisory',
		...overrides
	};
}

function callGet() {
	return GET({ platform: { env } } as never);
}

function callFile(body: unknown, headers: Record<string, string> = {}) {
	const request = new Request('https://registry.test/api/v0/advisories', {
		method: 'POST',
		headers: {
			'content-type': 'application/json',
			'X-RNX-Protocol': '0',
			Authorization: 'Bearer admin',
			...headers
		},
		body: JSON.stringify(body)
	});
	return POST({ request, platform: { env } } as never);
}

function callWithdraw(id: string) {
	const request = new Request(`https://registry.test/api/v0/advisories/${id}/withdraw`, {
		method: 'POST',
		headers: { 'X-RNX-Protocol': '0', Authorization: 'Bearer admin' },
		body: '{}'
	});
	return withdrawPOST({ request, params: { id }, platform: { env } } as never);
}

before(async () => {
	fake = await startFakeLibsql();
	env = { ...fake.env, ADVISORY_SIGNING_KEY: SEED, ADMIN_TOKEN: 'admin' };
	resetOperatorKeyCache();
});

after(async () => {
	await fake.close();
});

describe('advisory canonical bytes', () => {
	it('pins the cross-language golden vector', () => {
		const bytes = feedCanonicalBytes([
			{
				id: 'ADV-1',
				package: '@acme/widget',
				affected: '^1.0.0',
				artifacts: [],
				severity: 'warning',
				title: 'Bad "quotes" \\ test',
				detail: 'line1\nline2\té',
				state: 'active',
				createdAt: 1,
				updatedAt: 2
			}
		]);
		assert.equal(
			Buffer.from(bytes).toString('utf8'),
			'{"advisories":[{"affected":"^1.0.0","artifacts":[],"detail":"line1\\nline2\\té","id":"ADV-1","package":"@acme/widget","severity":"warning","title":"Bad \\"quotes\\" \\\\ test"}]}'
		);
	});

	it('sorts advisories by id and pins by artifact', () => {
		const a = {
			id: 'ADV-2',
			package: '@acme/widget',
			affected: '*',
			artifacts: [PIN],
			severity: 'critical' as const,
			title: 't',
			detail: '',
			state: 'active' as const,
			createdAt: 1,
			updatedAt: 1
		};
		const b = { ...a, id: 'ADV-1', artifacts: [] };
		const one = Buffer.from(feedCanonicalBytes([a, b])).toString('utf8');
		const two = Buffer.from(feedCanonicalBytes([b, a])).toString('utf8');
		assert.equal(one, two);
		assert.ok(one.indexOf('"id":"ADV-1"') < one.indexOf('"id":"ADV-2"'));
		assert.ok(one.includes(`"artifacts":["${PIN}"]`));
	});
});

describe('advisory signing', () => {
	it('round-trips sign and verify', async () => {
		const advisories = [
			{
				id: 'ADV-9',
				package: '@acme/widget',
				affected: '>=1.0.0',
				artifacts: [],
				severity: 'info' as const,
				title: 't',
				detail: 'd',
				state: 'active' as const,
				createdAt: 1,
				updatedAt: 1
			}
		];
		const feed = await signFeed(env, advisories);
		assert.equal(feed.advisories.length, 1);
		assert.equal(feed.keyId, (await operatorKey(env)).keyId);
		assert.equal(feed.publicKey, Buffer.from((await operatorKey(env)).publicKeyRaw).toString('hex'));
		assert.ok(await verifyFeedSignature(feed));
	});

	it('rejects tampered payloads, wrong keys, and key id mismatch', async () => {
		const feed = await signFeed(env, []);
		assert.ok(await verifyFeedSignature(feed));
		const tampered: SignedFeed = {
			...feed,
			advisories: [
				{
					id: 'ADV-X',
					package: '@acme/widget',
					affected: '*',
					artifacts: [],
					severity: 'critical',
					title: 'forged',
					detail: '',
					state: 'active',
					createdAt: 1,
					updatedAt: 1
				}
			]
		};
		assert.equal(await verifyFeedSignature(tampered), false);
		assert.equal(await verifyFeedSignature({ ...feed, signature: '00'.repeat(64) }), false);
		assert.equal(await verifyFeedSignature({ ...feed, keyId: 'deadbeefdeadbeef' }), false);
		assert.equal(await verifyFeedSignature({ ...feed, publicKey: 'zz' }), false);
		const other = await keyIdForRawPublicKey(randomBytes(32));
		assert.equal(await verifyFeedSignature({ ...feed, keyId: other }), false);
	});
});

describe('advisory validation', () => {
	it('rejects bad severity, range, pin, name, and empty title', async () => {
		assert.match((await validateAdvisoryInput(filedInput('V-1', { severity: 'fatal' }))).error ?? '', /severity/);
		assert.match((await validateAdvisoryInput(filedInput('V-2', { affected: '<1.0.0' }))).error ?? '', /range/);
		assert.match(
			(await validateAdvisoryInput(filedInput('V-3', { artifacts: ['sha256:xyz'] }))).error ?? '',
			/artifact pin/
		);
		assert.match((await validateAdvisoryInput(filedInput('V-4', { package: 'no spaces' }))).error ?? '', /package name/);
		assert.match((await validateAdvisoryInput(filedInput('V-5', { title: '  ' }))).error ?? '', /title/);
		assert.match((await validateAdvisoryInput(filedInput('bad id', {}))).error ?? '', /advisory id/);
		assert.match(
			(await validateAdvisoryInput(filedInput('V-7', { title: 'ok', detail: 'lone \udc00 here' }))).error ?? '',
			/valid unicode/
		);
	});

	it('accepts a pin-scoped critical advisory', async () => {
		const { advisory, error } = await validateAdvisoryInput(
			filedInput('V-6', { severity: 'critical', artifacts: [PIN.toUpperCase()] })
		);
		assert.equal(error, undefined);
		assert.deepEqual(advisory?.artifacts, [PIN]);
		assert.equal(advisory?.affected, '^1.0.0');
	});
});

describe('advisory store and feed route', () => {
	it('files, serves, and withdraws through the operator routes', async () => {
		const filed = await callFile(filedInput('ADV-101', { severity: 'critical', detail: 'hold' }));
		assert.equal(filed.status, 201);
		const dup = await callFile(filedInput('ADV-101'));
		assert.equal(dup.status, 409);
		const listed = await listActiveAdvisories(env);
		assert.ok(listed.some((a) => a.id === 'ADV-101' && a.severity === 'critical'));
		const res = await callGet();
		assert.equal(res.status, 200);
		assert.equal(res.headers.get('rnx-registry-spec'), '0');
		const feed = (await res.json()) as SignedFeed;
		assert.ok(feed.advisories.some((a) => a.id === 'ADV-101'));
		assert.ok(await verifyFeedSignature(feed));
		const out = await callWithdraw('ADV-101');
		assert.equal(out.status, 200);
		assert.equal((await listActiveAdvisories(env)).some((a) => a.id === 'ADV-101'), false);
		const again = await callGet();
		const feed2 = (await again.json()) as SignedFeed;
		assert.equal(feed2.advisories.some((a) => a.id === 'ADV-101'), false);
		assert.ok(verifyFeedSignature(feed2));
		const missing = await callWithdraw('ADV-101');
		assert.equal(missing.status, 404);
	});

	it('refuses operator routes without the admin token or protocol header', async () => {
		const noAuth = await callFile(filedInput('ADV-102'), { Authorization: 'Bearer wrong' });
		assert.equal(noAuth.status, 401);
		const noProto = await callFile(filedInput('ADV-102'), { 'X-RNX-Protocol': '9' });
		assert.equal(noProto.status, 400);
		const badBody = await callFile({ package: '@acme/widget', severity: 'warning' });
		assert.equal(badBody.status, 400);
	});

	it('files advisories through the store and records the audit trail', async () => {
		const a = await fileAdvisory(env, filedInput('ADV-103', { artifacts: [PIN] }));
		assert.deepEqual(a.artifacts, [PIN]);
		const rows = fake.db.prepare("SELECT action FROM audit_log WHERE details_json LIKE '%ADV-103%'").all() as Array<{
			action: string;
		}>;
		assert.ok(rows.some((r) => r.action === 'advisory-file'));
		assert.equal(await withdrawAdvisory(env, 'ADV-NOPE'), null);
	});
});
