import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { isTransientStoreError } from '../src/lib/server/registry.ts';

describe('isTransientStoreError', () => {
	it('flags transport and store failures as retryable', () => {
		for (const m of [
			'LibsqlError: SERVER_ERROR: Server returned HTTP status 503',
			'fetch failed: socket hang up',
			'request timed out after 30s',
			'connection reset by peer',
			'SQLITE_BUSY: database is locked',
			'service temporarily unavailable'
		]) {
			assert.equal(isTransientStoreError(m), true, m);
		}
	});

	it('leaves deterministic resolve errors alone', () => {
		for (const m of [
			'package @acme/widget does not exist',
			'no version of @acme/widget satisfies ^2.0.0 (have 1.0.0)',
			'conflicting ranges for @acme/widget',
			'dependency cycle: a -> b -> a',
			'invalid package name: foo',
			'registry resolved no version for package `@acme/widget`'
		]) {
			assert.equal(isTransientStoreError(m), false, m);
		}
	});
});
