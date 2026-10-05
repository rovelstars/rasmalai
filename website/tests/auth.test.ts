import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	validateUsername,
	hashPassword,
	verifyPassword,
	readSessionCookie,
	sessionCookie,
	clearSessionCookie,
	requirePepper
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
		const secure = sessionCookie('abc', 100, true);
		assert.match(secure, /HttpOnly/);
		assert.match(secure, /Secure/);
		assert.match(secure, /SameSite=Lax/);
		assert.equal(readSessionCookie(`other=1; ${secure.split(';')[0]}`), 'abc');
		assert.match(clearSessionCookie(true), /Max-Age=0/);
		assert.doesNotMatch(sessionCookie('abc', 100, false), /Secure/);
	});

	it('rejects bad salts closed and gates the pepper', async () => {
		assert.equal(await verifyPassword('x', 'pepper', { hash: '00', salt: 'not hex!!', params: '{}' }), false);
		assert.throws(() => requirePepper({}), /SESSION_PEPPER/);
		assert.equal(requirePepper({ RNX_ALLOW_NO_PEPPER: '1' }), '');
		assert.equal(requirePepper({ SESSION_PEPPER: 'p' }), 'p');
	});
});
