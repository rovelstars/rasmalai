import { argon2id } from 'hash-wasm';
import type { BatchItem } from 'drizzle-orm/batch';
import { and, count, eq, gt, lt, sql, sum } from 'drizzle-orm';
import { getDdb } from './db.js';
import * as s from './schema.js';

export const SESSION_COOKIE = 'rnx_session';
const SESSION_DAYS = 30;
const USERNAME_RE = /^[a-z0-9][a-z0-9-]{0,31}$/;
const RESERVED_USERNAMES = ['admin', 'root', 'system', 'rnx', 'std', 'org'];

export interface AuthUser {
	id: number;
	username: string;
	isAdmin: boolean;
}

export function validateUsername(username: string): string | null {
	if (!USERNAME_RE.test(username)) return 'lowercase letters, digits, hyphens (max 32)';
	if (RESERVED_USERNAMES.includes(username)) return 'username is reserved';
	return null;
}

export function safeEqual(a: string, b: string): boolean {
	if (a.length !== b.length) return false;
	let diff = 0;
	for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
	return diff === 0;
}

async function sha256Hex(text: string): Promise<string> {
	const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
	return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

export interface PasswordRow {
	hash: string;
	salt: string;
	params: string;
}

const ARGON_PARAMS = { iterations: 2, memorySize: 19456, parallelism: 1 };

export async function hashPassword(password: string, pepper: string): Promise<PasswordRow> {
	const salt = new Uint8Array(16);
	crypto.getRandomValues(salt);
	const saltHex = [...salt].map((b) => b.toString(16).padStart(2, '0')).join('');
	const hash = await argon2id({
		password,
		secret: pepper,
		salt,
		iterations: ARGON_PARAMS.iterations,
		memorySize: ARGON_PARAMS.memorySize,
		parallelism: ARGON_PARAMS.parallelism,
		hashLength: 32,
		outputType: 'hex'
	});
	return { hash, salt: saltHex, params: JSON.stringify({ algo: 'argon2id', ...ARGON_PARAMS, pepperVer: pepper ? 1 : 0 }) };
}

export async function verifyPassword(password: string, pepper: string, row: PasswordRow): Promise<boolean> {
	let params: { iterations: number; memorySize: number; parallelism: number };
	try {
		params = JSON.parse(row.params) as { iterations: number; memorySize: number; parallelism: number };
		if (!Number.isInteger(params.iterations) || !Number.isInteger(params.memorySize) || !Number.isInteger(params.parallelism)) {
			return false;
		}
	} catch {
		return false;
	}
	const groups = /^[0-9a-fA-F]+$/.test(row.salt) && row.salt.length % 2 === 0 ? row.salt.match(/../g) : null;
	if (!groups || groups.length === 0) return false;
	const salt = new Uint8Array(groups.map((h) => parseInt(h, 16)));
	const hash = await argon2id({
		password,
		secret: pepper,
		salt,
		iterations: params.iterations,
		memorySize: params.memorySize,
		parallelism: params.parallelism,
		hashLength: 32,
		outputType: 'hex'
	});
	return safeEqual(hash, row.hash);
}

export function sessionCookie(token: string, maxAge: number, secure: boolean): string {
	return `${SESSION_COOKIE}=${token}; Path=/; HttpOnly;${secure ? ' Secure;' : ''} SameSite=Lax; Max-Age=${maxAge}`;
}

export function clearSessionCookie(secure: boolean): string {
	return `${SESSION_COOKIE}=; Path=/; HttpOnly;${secure ? ' Secure;' : ''} SameSite=Lax; Max-Age=0`;
}

export function readSessionCookie(header: string | null): string {
	if (!header) return '';
	for (const part of header.split(';')) {
		const [k, ...v] = part.trim().split('=');
		if (k === SESSION_COOKIE) return v.join('=');
	}
	return '';
}

export function requirePepper(env: Record<string, string | undefined>): string {
	const pepper = env['SESSION_PEPPER'] ?? '';
	if (!pepper && env['RNX_ALLOW_NO_PEPPER'] !== '1') {
		throw new Error('SESSION_PEPPER is not configured');
	}
	return pepper;
}

export async function signup(
	env: Record<string, string | undefined>,
	username: string,
	password: string,
	disclaimerAck: boolean
): Promise<AuthUser> {
	const problem = validateUsername(username);
	if (problem) throw new Error(`invalid username: ${problem}`);
	if (password.length < 12) throw new Error('password must be at least 12 characters');
	if (!disclaimerAck) throw new Error('testing-phase deletion disclaimer must be accepted');
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	const pepper = requirePepper(env);
	const row = await hashPassword(password, pepper);
	const now = Math.floor(Date.now() / 1000);
	try {
		// last_insert_rowid() resolves on the batch connection, so the org
		// row always points at the user row from the statement above.
		await db.batch([
			db.insert(s.users).values({
				username,
				passwordHash: row.hash,
				passwordSalt: row.salt,
				passwordParams: row.params,
				disclaimerAck: 1,
				createdAt: now
			}),
			db.run(
				sql`INSERT INTO orgs (scope, owner_user_id, created_at) VALUES (${username}, last_insert_rowid(), ${now})`
			),
			db
				.insert(s.scopes)
				.values({ name: username, owner: username, reserved: 0, createdAt: now })
				.onConflictDoNothing()
		] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
		const idRow = await db
			.select({ id: s.users.id })
			.from(s.users)
			.where(eq(s.users.username, username))
			.get();
		return { id: Number(idRow?.id ?? 0), username, isAdmin: false };
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		if (msg.includes('UNIQUE')) throw new Error('username is taken');
		throw e;
	}
}

export async function login(
	env: Record<string, string | undefined>,
	username: string,
	password: string
): Promise<{ token: string; user: AuthUser }> {
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	const r = await db
		.select({
			id: s.users.id,
			username: s.users.username,
			passwordHash: s.users.passwordHash,
			passwordSalt: s.users.passwordSalt,
			passwordParams: s.users.passwordParams,
			isAdmin: s.users.isAdmin
		})
		.from(s.users)
		.where(eq(s.users.username, username))
		.get();
	const pepper = requirePepper(env);
	let ok = false;
	if (r) {
		ok = await verifyPassword(password, pepper, {
			hash: r.passwordHash,
			salt: r.passwordSalt,
			params: r.passwordParams
		});
	} else {
		const salt = new Uint8Array(16);
		crypto.getRandomValues(salt);
		await argon2id({
			password,
			secret: pepper,
			salt,
			iterations: ARGON_PARAMS.iterations,
			memorySize: ARGON_PARAMS.memorySize,
			parallelism: ARGON_PARAMS.parallelism,
			hashLength: 32,
			outputType: 'hex'
		});
	}
	if (!r || !ok) throw new Error('invalid username or password');
	const bytes = new Uint8Array(32);
	crypto.getRandomValues(bytes);
	const token = [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
	const now = Math.floor(Date.now() / 1000);
	await db.insert(s.sessions).values({
		tokenHash: await sha256Hex(token),
		userId: r.id,
		createdAt: now,
		expiresAt: now + SESSION_DAYS * 86400,
		revokedAt: 0
	});
	return { token, user: { id: r.id, username: r.username, isAdmin: r.isAdmin === 1 } };
}

export async function sessionUser(
	env: Record<string, string | undefined>,
	token: string
): Promise<AuthUser | null> {
	if (!token) return null;
	const db = await getDdb(env);
	if (!db) return null;
	const r = await db
		.select({ id: s.users.id, username: s.users.username, isAdmin: s.users.isAdmin })
		.from(s.sessions)
		.innerJoin(s.users, eq(s.users.id, s.sessions.userId))
		.where(
			and(
				eq(s.sessions.tokenHash, await sha256Hex(token)),
				eq(s.sessions.revokedAt, 0),
				gt(s.sessions.expiresAt, Math.floor(Date.now() / 1000))
			)
		)
		.get();
	if (!r) return null;
	return { id: r.id, username: r.username, isAdmin: r.isAdmin === 1 };
}

export async function logout(env: Record<string, string | undefined>, token: string): Promise<void> {
	const db = await getDdb(env);
	if (!db || !token) return;
	await db
		.update(s.sessions)
		.set({ revokedAt: Math.floor(Date.now() / 1000) })
		.where(eq(s.sessions.tokenHash, await sha256Hex(token)));
}

export async function userScopes(env: Record<string, string | undefined>, userId: number): Promise<string[]> {
	const db = await getDdb(env);
	if (!db) return [];
	const rows = await db
		.select({ scope: s.orgs.scope })
		.from(s.orgs)
		.where(eq(s.orgs.ownerUserId, userId));
	return rows.map((r) => r.scope);
}

export function clientIp(headers: Headers): string {
	const cf = headers.get('CF-Connecting-IP');
	if (cf && cf.trim()) return cf.trim();
	const fwd = headers.get('X-Forwarded-For');
	if (fwd && fwd.trim()) return fwd.split(',')[0].trim() || 'unknown';
	return 'unknown';
}

export function checkBrowserOrigin(request: Request, url: URL): boolean {
	const origin = request.headers.get('Origin');
	if (origin && origin.trim()) {
		try {
			return new URL(origin).origin === url.origin;
		} catch {
			return false;
		}
	}
	const referer = request.headers.get('Referer');
	if (referer && referer.trim()) {
		try {
			return new URL(referer).origin === url.origin;
		} catch {
			return false;
		}
	}
	return false;
}

const AUTH_WINDOW_SECONDS = 15 * 60;
export const AUTH_MAX_ATTEMPTS = 5;
const AUTH_PRUNE_SECONDS = 60 * 60;

export function authBlockedForCount(count: number): boolean {
	return count > AUTH_MAX_ATTEMPTS;
}

export async function authFailureCount(
	env: Record<string, string | undefined>,
	username: string
): Promise<number> {
	const db = await getDdb(env);
	if (!db) return 0;
	const now = Math.floor(Date.now() / 1000);
	const row = await db
		.select({ n: sum(s.authAttempts.attempts) })
		.from(s.authAttempts)
		.where(and(eq(s.authAttempts.username, username), gt(s.authAttempts.windowStart, now - AUTH_WINDOW_SECONDS)))
		.get();
	return Number(row?.n ?? 0);
}

export async function isAuthBlocked(
	env: Record<string, string | undefined>,
	_ip: string,
	username: string
): Promise<boolean> {
	const db = await getDdb(env);
	if (!db) return false;
	const now = Math.floor(Date.now() / 1000);
	await db.delete(s.authAttempts).where(lt(s.authAttempts.windowStart, now - AUTH_PRUNE_SECONDS));
	return authBlockedForCount(await authFailureCount(env, username));
}

export async function auth_attempts(
	env: Record<string, string | undefined>,
	ip: string,
	username: string,
	ok: boolean
): Promise<boolean> {
	const db = await getDdb(env);
	if (!db) return true;
	const now = Math.floor(Date.now() / 1000);
	await db.delete(s.authAttempts).where(lt(s.authAttempts.windowStart, now - AUTH_PRUNE_SECONDS));
	if (ok) {
		await db.delete(s.authAttempts).where(eq(s.authAttempts.username, username));
		return true;
	}
	const cutoff = now - AUTH_WINDOW_SECONDS;
	const applied = await db.batch([
		db
			.insert(s.authAttempts)
			.values({ ip, username, attempts: 1, windowStart: now })
			.onConflictDoUpdate({
				target: [s.authAttempts.ip, s.authAttempts.username],
				set: {
					attempts:
						sql`CASE WHEN ${s.authAttempts.windowStart} < ${cutoff} THEN 1 ELSE ${s.authAttempts.attempts} + 1 END`,
					windowStart:
						sql`CASE WHEN ${s.authAttempts.windowStart} < ${cutoff} THEN ${now} ELSE ${s.authAttempts.windowStart} END`
				}
			}),
		db
			.select({ n: sum(s.authAttempts.attempts) })
			.from(s.authAttempts)
			.where(and(eq(s.authAttempts.username, username), gt(s.authAttempts.windowStart, cutoff)))
	] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	const counted = applied[1]?.[0]?.n ?? 0;
	return !authBlockedForCount(Number(counted));
}
