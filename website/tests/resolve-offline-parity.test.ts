import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import {
	resolveGraph,
	yankVersion,
	recordTombstone,
	clearVersionIndex
} from '../src/lib/server/db.js';
import { seedPackage, seedPayload, type CanonicalPublishPayload } from './publish-helpers.js';

// Paired with the CLI offline selector (`judge_cached` in
// compiler/frontend/src/fetch.rs, tests `offline_float_*`): the same
// inventories must pick the same versions. The engine below is the
// toolchain the CLI resolves with (compiler/frontend/Cargo.toml keeps
// `version = "0.2.1"`); if that version moves, update ENGINE here and the
// CLI vectors stay paired.

const ENGINE = '0.2.1';

let fake: FakeLibsql;

function payload(
	name: string,
	version: string,
	overrides: Partial<CanonicalPublishPayload> = {}
): CanonicalPublishPayload {
	return seedPayload(name, version, overrides);
}

async function seedParity() {
	await seedPackage(fake.env, payload('@acme/parity', '1.2.0'));
	await seedPackage(fake.env, payload('@acme/parity', '1.2.5'));
	await seedPackage(fake.env, payload('@acme/parity', '1.3.0'));
	await seedPackage(fake.env, payload('@acme/parity', '2.0.0', { engineRange: '>=99.0.0' }));
	await seedPackage(fake.env, payload('@acme/parity', '1.2.3-beta.1'));
}

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

beforeEach(() => {
	clearVersionIndex();
	fake.reset();
});

describe('offline selection parity (server side)', () => {
	it('picks the engine-compatible max for each range', async () => {
		await seedParity();
		clearVersionIndex();
		const cases: Array<[string, string]> = [
			['*', '1.3.0'],
			['^1.0.0', '1.3.0'],
			['~1.2.0', '1.2.5'],
			['>=1.2.0', '1.3.0'],
			['^1.2.3-beta.1', '1.3.0']
		];
		for (const [range, want] of cases) {
			clearVersionIndex();
			const out = await resolveGraph(fake.env, { '@acme/parity': range }, [], {
				engine: ENGINE
			});
			assert.equal(out.resolved['@acme/parity'], want, `range ${range}`);
		}
	});

	it('answers an engine mismatch as unsatisfiable, naming package, range and engine', async () => {
		await seedParity();
		clearVersionIndex();
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/parity': '2.0.0' }, [], { engine: ENGINE }),
			(err: unknown) => {
				const msg = String((err as Error)?.message ?? err);
				assert.match(msg, /@acme\/parity/);
				assert.match(msg, /2\.0\.0/);
				assert.match(msg, /for engine/);
				assert.match(msg, /0\.2\.1/);
				assert.match(msg, /satisfies/);
				return true;
			}
		);
	});

	it('moves past a tombstoned version the candidate list still names', async () => {
		await seedPackage(fake.env, payload('@acme/ptomb', '1.2.0'));
		await seedPackage(fake.env, payload('@acme/ptomb', '1.3.0'));
		await recordTombstone(fake.env, '@acme/ptomb', '1.3.0', 'malware');
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/ptomb': '^1.0.0' }, [], {
			engine: ENGINE
		});
		assert.equal(out.resolved['@acme/ptomb'], '1.2.0');
	});

	it('keeps serving a pinned yanked version, flagged', async () => {
		await seedPackage(fake.env, payload('@acme/ppin', '1.2.0'));
		await seedPackage(fake.env, payload('@acme/ppin', '1.3.0'));
		await yankVersion(fake.env, '@acme/ppin', '1.3.0');
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/ppin': '1.3.0' }, [], {
			engine: ENGINE
		});
		assert.equal(out.resolved['@acme/ppin'], '1.3.0');
		assert.equal(out.levels[0][0].yanked, true);
	});
});
