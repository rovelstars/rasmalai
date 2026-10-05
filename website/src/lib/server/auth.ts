import { argon2id } from 'hash-wasm';
import { getClient, ensureSchema } from './db.js';

// TEMPORARY accounts system: username+password with single-member orgs,
// good enough for registry testing. MUST be replaced by Rovel Stars'
// unified accounts system (shared identity, OAuth, teams). Do not grow
// this file toward a real identity product.
export const SESSION_COOKIE = 'rnx_session';
const SESSION_DAYS = 30;
const USERNAME_RE = /^[a-z0-9][a-z0-9-]{0,31}$/;
const RESERVED_USERNAMES = ['admin', 'root', 'system', 'rnx', 'std'];

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
	const db = getClient(env);
	if (!db) throw new Error('missing database');
	await ensureSchema(db);
	const pepper = requirePepper(env);
	const row = await hashPassword(password, pepper);
	const now = Math.floor(Date.now() / 1000);
	try {
		// last_insert_rowid() resolves on the batch connection, so the org
		// row always points at the user row from the statement above.
		await db.batch([
			{
				sql: `INSERT INTO users (username, password_hash, password_salt, password_params, disclaimer_ack, created_at)
				      VALUES (?, ?, ?, ?, 1, ?)`,
				args: [username, row.hash, row.salt, row.params, now]
			},
			{
				sql: `INSERT INTO orgs (scope, owner_user_id, created_at) VALUES (?, last_insert_rowid(), ?)`,
				args: [username, now]
			},
			{
				sql: `INSERT INTO scopes (name, owner, reserved, created_at) VALUES (?, ?, 0, ?)
				      ON CONFLICT(name) DO NOTHING`,
				args: [username, username, now]
			}
		]);
		const idRs = await db.execute({
			sql: 'SELECT id FROM users WHERE username = ?',
			args: [username]
		});
		return { id: Number(idRs.rows[0]?.['id'] ?? 0), username, isAdmin: false };
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
	const db = getClient(env);
	if (!db) throw new Error('missing database');
	await ensureSchema(db);
	const rs = await db.execute({
		sql: 'SELECT id, username, password_hash, password_salt, password_params, is_admin FROM users WHERE username = ?',
		args: [username]
	});
	const r = rs.rows[0] as Record<string, unknown> | undefined;
	const pepper = requirePepper(env);
	let ok = false;
	if (r) {
		ok = await verifyPassword(password, pepper, {
			hash: String(r['password_hash']),
			salt: String(r['password_salt']),
			params: String(r['password_params'])
		});
	}
	if (!r || !ok) throw new Error('invalid username or password');
	const bytes = new Uint8Array(32);
	crypto.getRandomValues(bytes);
	const token = [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
	const now = Math.floor(Date.now() / 1000);
	await db.execute({
		sql: `INSERT INTO sessions (token_hash, user_id, created_at, expires_at, revoked_at)
		      VALUES (?, ?, ?, ?, 0)`,
		args: [await sha256Hex(token), Number(r['id']), now, now + SESSION_DAYS * 86400]
	});
	return { token, user: { id: Number(r['id']), username: String(r['username']), isAdmin: Number(r['is_admin']) === 1 } };
}

export async function sessionUser(
	env: Record<string, string | undefined>,
	token: string
): Promise<AuthUser | null> {
	if (!token) return null;
	const db = getClient(env);
	if (!db) return null;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT u.id AS id, u.username AS username, u.is_admin AS is_admin
		      FROM sessions s JOIN users u ON u.id = s.user_id
		      WHERE s.token_hash = ? AND s.revoked_at = 0 AND s.expires_at > ?`,
		args: [await sha256Hex(token), Math.floor(Date.now() / 1000)]
	});
	const r = rs.rows[0] as Record<string, unknown> | undefined;
	if (!r) return null;
	return { id: Number(r['id']), username: String(r['username']), isAdmin: Number(r['is_admin']) === 1 };
}

export async function logout(env: Record<string, string | undefined>, token: string): Promise<void> {
	const db = getClient(env);
	if (!db || !token) return;
	await ensureSchema(db);
	await db.execute({
		sql: 'UPDATE sessions SET revoked_at = ? WHERE token_hash = ?',
		args: [Math.floor(Date.now() / 1000), await sha256Hex(token)]
	});
}

export async function userScopes(env: Record<string, string | undefined>, userId: number): Promise<string[]> {
	const db = getClient(env);
	if (!db) return [];
	await ensureSchema(db);
	const rs = await db.execute({
		sql: 'SELECT scope FROM orgs WHERE owner_user_id = ?',
		args: [userId]
	});
	return rs.rows.map((r) => String(r['scope']));
}

export function clientIp(headers: Headers): string {
	const cf = headers.get('CF-Connecting-IP');
	if (cf && cf.trim()) return cf.trim();
	const fwd = headers.get('X-Forwarded-For');
	if (fwd && fwd.trim()) return fwd.split(',')[0].trim() || 'unknown';
	return 'unknown';
}

const AUTH_WINDOW_SECONDS = 15 * 60;
const AUTH_MAX_FAILURES = 5;
const AUTH_PRUNE_SECONDS = 60 * 60;

export async function isAuthBlocked(
	env: Record<string, string | undefined>,
	ip: string,
	username: string
): Promise<boolean> {
	const db = getClient(env);
	if (!db) return false;
	await ensureSchema(db);
	const now = Math.floor(Date.now() / 1000);
	await db.execute({
		sql: 'DELETE FROM auth_attempts WHERE window_start < ?',
		args: [now - AUTH_PRUNE_SECONDS]
	});
	const rs = await db.execute({
		sql: 'SELECT attempts, window_start FROM auth_attempts WHERE ip = ? AND username = ?',
		args: [ip, username]
	});
	const row = rs.rows[0] as Record<string, unknown> | undefined;
	if (!row) return false;
	if (now - Number(row['window_start']) > AUTH_WINDOW_SECONDS) return false;
	return Number(row['attempts']) > AUTH_MAX_FAILURES;
}

export async function auth_attempts(
	env: Record<string, string | undefined>,
	ip: string,
	username: string,
	ok: boolean
): Promise<boolean> {
	const db = getClient(env);
	if (!db) return true;
	await ensureSchema(db);
	const now = Math.floor(Date.now() / 1000);
	await db.execute({
		sql: 'DELETE FROM auth_attempts WHERE window_start < ?',
		args: [now - AUTH_PRUNE_SECONDS]
	});
	if (ok) {
		await db.execute({
			sql: 'DELETE FROM auth_attempts WHERE ip = ? AND username = ?',
			args: [ip, username]
		});
		return true;
	}
	await db.execute({
		sql: `INSERT INTO auth_attempts (ip, username, attempts, window_start)
		      VALUES (?, ?, 1, ?)
		      ON CONFLICT(ip, username) DO UPDATE SET
		        attempts = CASE WHEN window_start < ? THEN 1 ELSE attempts + 1 END,
		        window_start = CASE WHEN window_start < ? THEN ? ELSE window_start END`,
		args: [ip, username, now, now - AUTH_WINDOW_SECONDS, now - AUTH_WINDOW_SECONDS, now]
	});
	return !(await isAuthBlocked(env, ip, username));
}
