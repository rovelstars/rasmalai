import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	validateUsername,
	hashPassword,
	verifyPassword,
	readSessionCookie,
	sessionCookie,
	clearSessionCookie,
	requirePepper,
	clientIp,
	checkBrowserOrigin,
	AUTH_MAX_ATTEMPTS,
	authBlockedForCount
} from '../src/lib/server/auth.js';

describe('auth', () => {
	it('validates usernames', () => {
		assert.equal(validateUsername('alice'), null);
		assert.equal(validateUsername('admin') !== null, true);
		assert.equal(validateUsername('org') !== null, true);
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

	it('parses client IP for forensics only (throttle keys by username)', () => {
		assert.equal(clientIp(new Headers({ 'CF-Connecting-IP': '1.2.3.4' })), '1.2.3.4');
		assert.equal(clientIp(new Headers({ 'X-Forwarded-For': '5.6.7.8, 9.9.9.9' })), '5.6.7.8');
		assert.equal(clientIp(new Headers({})), 'unknown');
	});

	it('checks browser origin for session-cookie mutations', () => {
		const url = new URL('https://example.com/api/packages');
		const withOrigin = new Request('https://example.com/api/packages', {
			method: 'POST',
			headers: { Origin: 'https://example.com' }
		});
		assert.equal(checkBrowserOrigin(withOrigin, url), true);
		const badOrigin = new Request('https://example.com/api/packages', {
			method: 'POST',
			headers: { Origin: 'https://evil.com' }
		});
		assert.equal(checkBrowserOrigin(badOrigin, url), false);
		const withReferer = new Request('https://example.com/api/packages', {
			method: 'POST',
			headers: { Referer: 'https://example.com/packages/foo' }
		});
		assert.equal(checkBrowserOrigin(withReferer, url), true);
		const badReferer = new Request('https://example.com/api/packages', {
			method: 'POST',
			headers: { Referer: 'https://evil.com/x' }
		});
		assert.equal(checkBrowserOrigin(badReferer, url), false);
		const absent = new Request('https://example.com/api/packages', { method: 'POST' });
		assert.equal(checkBrowserOrigin(absent, url), false);
	});

	it('blocks the 6th attempt (attempts 1-5 allowed)', () => {
		assert.equal(AUTH_MAX_ATTEMPTS, 5);
		for (let n = 1; n <= 5; n++) assert.equal(authBlockedForCount(n), false, `count ${n}`);
		assert.equal(authBlockedForCount(6), true);
		assert.equal(authBlockedForCount(100), true);
	});
});
