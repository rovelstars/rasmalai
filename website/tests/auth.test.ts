import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	validateUsername,
	hashPassword,
	verifyPassword,
	readSessionCookie,
	sessionCookie,
	clearSessionCookie
} from '../src/lib/server/auth.js';

describe('auth', () => {
	it('validates usernames', () => {
		assert.equal(validateUsername('alice'), null);
		assert.equal(validateUsername('admin') !== null, true);
		assert.equal(validateUsername('Bad Name') !== null, true);
	});

	it('hashes and verifies passwords', async () => {
		const row = await hashPassword('correct-horse-123', 'pepper');
		assert.equal(await verifyPassword('correct-horse-123', 'pepper', row), true);
		assert.equal(await verifyPassword('wrong-password!', 'pepper', row), false);
		assert.equal(await verifyPassword('correct-horse-123', 'other', row), false);
	});

	it('round-trips session cookies', () => {
		const set = sessionCookie('abc', 100);
		assert.match(set, /HttpOnly/);
		assert.match(set, /SameSite=Lax/);
		assert.equal(readSessionCookie(`other=1; ${set.split(';')[0]}`), 'abc');
		assert.match(clearSessionCookie(), /Max-Age=0/);
	});
});
