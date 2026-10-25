import { asc, eq } from 'drizzle-orm';
import type { BatchItem } from 'drizzle-orm/batch';
import { getDdb, parsePackageName } from './db.js';
import * as s from './schema.js';
import { isValidRange } from './registry.js';

type NodeCrypto = typeof import('node:crypto');
type KeyObject = ReturnType<NodeCrypto['createPublicKey']>;

let cryptoModule: NodeCrypto | null = null;

async function nodeCrypto(): Promise<NodeCrypto> {
	if (!cryptoModule) cryptoModule = await import('node:crypto');
	return cryptoModule;
}

export const ADVISORY_SEVERITIES = ['critical', 'warning', 'info'] as const;
export type AdvisorySeverity = (typeof ADVISORY_SEVERITIES)[number];

export interface Advisory {
	id: string;
	package: string;
	affected: string;
	artifacts: string[];
	severity: AdvisorySeverity;
	title: string;
	detail: string;
	state: 'active' | 'withdrawn';
	createdAt: number;
	updatedAt: number;
}

export interface AdvisoryInput {
	id?: string;
	package: string;
	affected?: string;
	artifacts?: string[];
	severity: string;
	title?: string;
	detail?: string;
}

export interface SignedFeed {
	advisories: Advisory[];
	keyId: string;
	publicKey: string;
	signature: string;
}

const ID_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const ARTIFACT_RE = /^sha256:[0-9a-f]{64}$/;
const SEED_RE = /^[0-9a-fA-F]{64}$/;
const PKCS8_PREFIX = '302e020100300506032b657004220420';
const SPKI_PREFIX = '302a300506032b6570032100';

function canonicalString(value: string): string {
	let out = '"';
	for (const ch of value) {
		const code = ch.codePointAt(0) ?? 0;
		if (ch === '"') out += '\\"';
		else if (ch === '\\') out += '\\\\';
		else if (code === 0x08) out += '\\b';
		else if (code === 0x09) out += '\\t';
		else if (code === 0x0a) out += '\\n';
		else if (code === 0x0c) out += '\\f';
		else if (code === 0x0d) out += '\\r';
		else if (code < 0x20) out += `\\u00${code.toString(16).padStart(2, '0')}`;
		else out += ch;
	}
	return out + '"';
}

function canonicalAdvisory(a: Advisory): string {
	const pins = [...a.artifacts].sort().map(canonicalString).join(',');
	return (
		`{"affected":${canonicalString(a.affected)},` +
		`"artifacts":[${pins}],` +
		`"detail":${canonicalString(a.detail)},` +
		`"id":${canonicalString(a.id)},` +
		`"package":${canonicalString(a.package)},` +
		`"severity":${canonicalString(a.severity)},` +
		`"title":${canonicalString(a.title)}}`
	);
}

export function feedCanonicalBytes(advisories: Advisory[]): Uint8Array {
	const sorted = [...advisories].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
	return new TextEncoder().encode(`{"advisories":[${sorted.map(canonicalAdvisory).join(',')}]}`);
}

function rawPublicKey(publicKey: KeyObject): Buffer {
	const der = publicKey.export({ format: 'der', type: 'spki' }) as Buffer;
	return der.subarray(der.length - 32);
}

export async function keyIdForRawPublicKey(raw: Uint8Array): Promise<string> {
	const nc = await nodeCrypto();
	return nc.createHash('sha256').update(raw).digest('hex').slice(0, 16);
}

interface OperatorKey {
	privateKey: KeyObject;
	publicKeyRaw: Buffer;
	keyId: string;
	ephemeral: boolean;
}

let cachedKey: { seed: string | null; key: OperatorKey } | null = null;

export async function operatorKey(env: Record<string, string | undefined>): Promise<OperatorKey> {
	const nc = await nodeCrypto();
	const seed = typeof env['ADVISORY_SIGNING_KEY'] === 'string' ? env['ADVISORY_SIGNING_KEY'].trim() : '';
	const cacheSeed = SEED_RE.test(seed) ? seed.toLowerCase() : null;
	if (cachedKey && cachedKey.seed === cacheSeed) return cachedKey.key;
	let privateKey: KeyObject;
	let ephemeral = false;
	if (cacheSeed) {
		privateKey = nc.createPrivateKey({
			key: Buffer.from(PKCS8_PREFIX + cacheSeed, 'hex'),
			format: 'der',
			type: 'pkcs8'
		});
	} else {
		privateKey = nc.generateKeyPairSync('ed25519').privateKey;
		ephemeral = true;
	}
	const publicKey = nc.createPublicKey(privateKey);
	const publicKeyRaw = rawPublicKey(publicKey);
	const key = { privateKey, publicKeyRaw, keyId: await keyIdForRawPublicKey(publicKeyRaw), ephemeral };
	cachedKey = { seed: cacheSeed, key };
	return key;
}

export function resetOperatorKeyCache(): void {
	cachedKey = null;
}

export async function signFeed(
	env: Record<string, string | undefined>,
	advisories: Advisory[]
): Promise<SignedFeed> {
	const nc = await nodeCrypto();
	const key = await operatorKey(env);
	const bytes = feedCanonicalBytes(advisories);
	const signature = nc.sign(null, bytes, key.privateKey);
	return {
		advisories: [...advisories].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)),
		keyId: key.keyId,
		publicKey: Buffer.from(key.publicKeyRaw).toString('hex'),
		signature: signature.toString('hex')
	};
}

export async function verifyFeedSignature(feed: SignedFeed): Promise<boolean> {
	try {
		const nc = await nodeCrypto();
		if (!/^[0-9a-fA-F]{64}$/.test(feed.publicKey)) return false;
		if (!/^[0-9a-fA-F]{128}$/.test(feed.signature)) return false;
		const publicKey = nc.createPublicKey({
			key: Buffer.from(SPKI_PREFIX + feed.publicKey.toLowerCase(), 'hex'),
			format: 'der',
			type: 'spki'
		});
		if ((await keyIdForRawPublicKey(rawPublicKey(publicKey))) !== feed.keyId) return false;
		return nc.verify(null, feedCanonicalBytes(feed.advisories), publicKey, Buffer.from(feed.signature, 'hex'));
	} catch {
		return false;
	}
}

export async function validateAdvisoryInput(
	input: AdvisoryInput
): Promise<{ advisory?: Advisory; error?: string }> {
	const now = Math.floor(Date.now() / 1000);
	const nc = await nodeCrypto();
	const id = (input.id ?? `adv-${nc.randomBytes(6).toString('hex')}`).trim();
	if (!ID_RE.test(id)) return { error: `invalid advisory id ${id}` };
	const full = (input.package ?? '').trim();
	if (!parsePackageName(full)) return { error: `invalid package name ${full}` };
	const affected = (input.affected ?? '*').trim() || '*';
	if (!isValidRange(affected)) {
		return { error: `affected is not a valid range (*, latest, X.Y.Z, ^, ~, >=)` };
	}
	const seen = new Set<string>();
	const artifacts: string[] = [];
	for (const pin of input.artifacts ?? []) {
		const p = String(pin).trim().toLowerCase();
		if (!ARTIFACT_RE.test(p)) return { error: `invalid artifact pin ${pin}` };
		if (!seen.has(p)) {
			seen.add(p);
			artifacts.push(p);
		}
	}
	artifacts.sort();
	if (!ADVISORY_SEVERITIES.includes(input.severity as AdvisorySeverity)) {
		return { error: `invalid severity ${input.severity} (critical, warning, info)` };
	}
	const title = (input.title ?? '').trim();
	const detail = (input.detail ?? '').trim();
	if (!title) return { error: 'advisory title is required' };
	if (/[\ud800-\udfff]/.test(title) || /[\ud800-\udfff]/.test(detail)) {
		return { error: 'advisory title and detail must be valid unicode' };
	}
	return {
		advisory: {
			id,
			package: full,
			affected,
			artifacts,
			severity: input.severity as AdvisorySeverity,
			title,
			detail,
			state: 'active',
			createdAt: now,
			updatedAt: now
		}
	};
}

function rowToAdvisory(row: typeof s.advisories.$inferSelect): Advisory {
	let artifacts: string[] = [];
	try {
		const parsed: unknown = JSON.parse(row.artifacts);
		if (Array.isArray(parsed)) artifacts = parsed.filter((p): p is string => typeof p === 'string');
	} catch {
		artifacts = [];
	}
	return {
		id: row.id,
		package: row.package,
		affected: row.affected,
		artifacts,
		severity: row.severity as AdvisorySeverity,
		title: row.title,
		detail: row.detail,
		state: row.state === 'withdrawn' ? 'withdrawn' : 'active',
		createdAt: row.createdAt,
		updatedAt: row.updatedAt
	};
}

export async function listActiveAdvisories(
	env: Record<string, string | undefined>
): Promise<Advisory[]> {
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	const rows = await db
		.select()
		.from(s.advisories)
		.where(eq(s.advisories.state, 'active'))
		.orderBy(asc(s.advisories.id));
	return rows.map(rowToAdvisory);
}

export async function fileAdvisory(
	env: Record<string, string | undefined>,
	input: AdvisoryInput
): Promise<Advisory> {
	const checked = await validateAdvisoryInput(input);
	if (!checked.advisory) throw new Error(checked.error ?? 'invalid advisory');
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	const a = checked.advisory;
	try {
		await db.batch([
			db.insert(s.advisories).values({
				id: a.id,
				package: a.package,
				affected: a.affected,
				artifacts: JSON.stringify(a.artifacts),
				severity: a.severity,
				title: a.title,
				detail: a.detail,
				state: 'active',
				createdAt: a.createdAt,
				updatedAt: a.updatedAt
			}),
			db.insert(s.auditLog).values({
				action: 'advisory-file',
				fullName: a.package,
				version: a.affected,
				detailsJson: JSON.stringify({ id: a.id, severity: a.severity }),
				createdAt: a.createdAt
			})
		] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	} catch {
		throw new Error(`advisory ${a.id} already exists`);
	}
	return a;
}

export async function withdrawAdvisory(
	env: Record<string, string | undefined>,
	id: string
): Promise<Advisory | null> {
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	const rows = await db.select().from(s.advisories).where(eq(s.advisories.id, id));
	const found = rows[0];
	if (!found || found.state !== 'active') return null;
	const now = Math.floor(Date.now() / 1000);
	await db.batch([
		db.update(s.advisories).set({ state: 'withdrawn', updatedAt: now }).where(eq(s.advisories.id, id)),
		db.insert(s.auditLog).values({
			action: 'advisory-withdraw',
			fullName: found.package,
			version: found.affected,
			detailsJson: JSON.stringify({ id }),
			createdAt: now
		})
	] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	return { ...rowToAdvisory(found), state: 'withdrawn', updatedAt: now };
}
