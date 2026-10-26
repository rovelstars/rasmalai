// A local stand-in for the libSQL HTTP endpoint, backed by node:sqlite.
//
// db.ts hard-wires `@libsql/client/web`, which rejects a `file:` URL, so
// resolveGraph cannot be pointed at a throwaway database from a test. This
// serves the Hrana v2 pipeline protocol that web client speaks, so a test
// drives the real drizzle/libsql code path, counts statements and round trips
// exactly, and exercises `changes()`-based batch semantics (publish
// idempotency) against a real SQLite engine.
import { DatabaseSync } from 'node:sqlite';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { BASE_SCHEMA } from '../src/lib/server/db.js';

type Json = Record<string, unknown>;

export interface FakeCounters {
	statements: number;
	roundTrips: number;
	rowsRead: number;
	bytesRead: number;
}

export interface FakeLibsql {
	url: string;
	env: Record<string, string | undefined>;
	counters: FakeCounters;
	statements: string[];
	reset(): void;
	close(): Promise<void>;
	db: DatabaseSync;
}

function encodeValue(v: unknown): Json {
	if (v === null || v === undefined) return { type: 'null' };
	if (typeof v === 'bigint') return { type: 'integer', value: String(v) };
	if (typeof v === 'number') {
		return Number.isInteger(v)
			? { type: 'integer', value: String(v) }
			: { type: 'float', value: v };
	}
	if (typeof v === 'boolean') return { type: 'integer', value: v ? '1' : '0' };
	if (v instanceof Uint8Array) return { type: 'blob', base64: Buffer.from(v).toString('base64') };
	return { type: 'text', value: String(v) };
}

function decodeArg(cell: Json): string | number | bigint | null | Buffer {
	const type = String(cell['type']);
	if (type === 'null') return null;
	if (type === 'integer') return BigInt(String(cell['value']));
	if (type === 'float') return Number(cell['value']);
	if (type === 'blob') return Buffer.from(String(cell['base64']), 'base64');
	return String(cell['value']);
}

export async function startFakeLibsql(): Promise<FakeLibsql> {
	return startFakeLibsqlWith({ schema: true });
}

// A database as a fresh deployment would find it: empty, with no tables
// at all. The first query runs the migration ladder, which has to create
// every table, backfill every additive column, and stamp the version.
export async function startEmptyFakeLibsql(): Promise<FakeLibsql> {
	return startFakeLibsqlWith({ schema: false });
}

async function startFakeLibsqlWith({ schema }: { schema: boolean }): Promise<FakeLibsql> {
	const db = new DatabaseSync(':memory:');
	db.exec('PRAGMA foreign_keys = ON');
	if (schema) {
		for (const stmt of BASE_SCHEMA.split(';')) {
			const sql = stmt.trim();
			if (sql) db.exec(sql);
		}
	}
	const counters: FakeCounters = { statements: 0, roundTrips: 0, rowsRead: 0, bytesRead: 0 };
	const statements: string[] = [];
	const sqlIds = new Map<number, string>();
	let inAutocommit = true;

	const runOne = (stmtJson: Json): Json => {
		const id = stmtJson['sql_id'];
		const sql =
			typeof stmtJson['sql'] === 'string'
				? (stmtJson['sql'] as string)
				: (sqlIds.get(Number(id)) ?? '');
		const args = ((stmtJson['args'] ?? []) as Json[]).map(decodeArg);
		counters.statements += 1;
		statements.push(sql);
		const prepared = db.prepare(sql);
		const cols = prepared.columns().map((c) => String(c.name));
		let rowObjects: Record<string, unknown>[] = [];
		let affected = 0;
		let lastRowid = 0;
		if (cols.length > 0) {
			rowObjects = prepared.all(...args) as Record<string, unknown>[];
			counters.rowsRead += rowObjects.length;
			// Bytes the client actually receives for the row values. A
			// statement that selects a large text column pays for it here,
			// which is what makes the cost of a wide projection visible.
			for (const row of rowObjects) {
				for (const col of cols) {
					const value = row[col];
					if (typeof value === 'string') counters.bytesRead += Buffer.byteLength(value);
					else if (value instanceof Uint8Array) counters.bytesRead += value.byteLength;
					else if (typeof value === 'bigint' || typeof value === 'number') counters.bytesRead += 8;
				}
			}
		} else {
			// node:sqlite only reports `changes` from run(); all() leaves it
			// undefined, and publish idempotency reads affected_row_count.
			const r = prepared.run(...args);
			affected = Number(r.changes ?? 0);
			lastRowid = Number(r.lastInsertRowid ?? 0);
		}
		const upper = sql.trim().toUpperCase();
		if (upper.startsWith('BEGIN')) inAutocommit = false;
		if (upper.startsWith('COMMIT') || upper.startsWith('ROLLBACK') || upper.startsWith('END')) {
			inAutocommit = true;
		}
		return {
			cols: cols.map((name) => ({ name, decltype: null })),
			// Rows are positional arrays in the protocol; column names carry the
			// mapping on the way back through ResultSet.
			rows: rowObjects.map((row) => cols.map((c) => encodeValue(row[c]))),
			affected_row_count: affected,
			last_insert_rowid: String(lastRowid)
		};
	};

	type Outcome = { ok: true; result: Json } | { ok: false; message: string };
	const runStep = (stmtJson: Json): Outcome => {
		try {
			return { ok: true, result: runOne(stmtJson) };
		} catch (e) {
			return { ok: false, message: e instanceof Error ? e.message : String(e) };
		}
	};

	// Batch conditions decide which steps run: a step guarded by
	// `ok(failedStep)` is skipped, which is what makes the
	// `INSERT ... WHERE changes() > 0` audit idiom and publish idempotency
	// behave the way they do against real libSQL.
	const condHolds = (cond: Json | undefined, ok: boolean[], depth: number): boolean => {
		if (cond === undefined || depth > 16) return true;
		const type = String(cond['type']);
		if (type === 'ok') return ok[Number(cond['step'])] === true;
		if (type === 'is_autocommit') return inAutocommit;
		if (type === 'not') return !condHolds(cond['cond'] as Json, ok, depth + 1);
		if (type === 'and') {
			return ((cond['conds'] ?? []) as Json[]).every((c) => condHolds(c, ok, depth + 1));
		}
		if (type === 'or') {
			return ((cond['conds'] ?? []) as Json[]).some((c) => condHolds(c, ok, depth + 1));
		}
		return true;
	};

	const handle = (requests: Json[]): Json[] =>
		requests.map((request) => {
			const type = String(request['type']);
			if (type === 'store_sql') {
				sqlIds.set(Number(request['sql_id']), String(request['sql'] ?? ''));
				return { type: 'ok', response: { type: 'store_sql' } };
			}
			if (type === 'close_sql') {
				sqlIds.delete(Number(request['sql_id']));
				return { type: 'ok', response: { type: 'close_sql' } };
			}
			if (type === 'close') return { type: 'ok', response: { type: 'close' } };
			if (type === 'execute') {
				const out = runStep(request['stmt'] as Json);
				if (!out.ok) return { type: 'error', error: { message: out.message, code: 'SQLITE_ERROR' } };
				return { type: 'ok', response: { type: 'execute', result: out.result } };
			}
			if (type === 'batch') {
				// A step is `{condition?, stmt}`; the condition guards the step,
				// so it must survive extraction.
				const rawSteps = ((request['batch'] as { steps: Json[] }).steps ?? []) as Json[];
				const steps = rawSteps.map((s) =>
					s['stmt'] !== undefined ? { ...(s['stmt'] as Json), condition: s['condition'] } : s
				);
				const okFlags: boolean[] = [];
				const stepResults: (Json | null)[] = [];
				const stepErrors: (Json | null)[] = [];
				for (let i = 0; i < steps.length; i++) {
					const step = steps[i] as Json;
					if (!condHolds(step['condition'] as Json | undefined, okFlags, 0)) {
						okFlags.push(false);
						stepResults.push(null);
						stepErrors.push(null);
						continue;
					}
					const out = runStep(step);
					okFlags.push(out.ok);
					stepResults.push(out.ok ? out.result : null);
					stepErrors.push(out.ok ? null : { message: out.message, code: 'SQLITE_ERROR' });
				}
				const firstError = stepErrors.find((e) => e !== null);
				if (firstError) {
					return { type: 'error', error: firstError };
				}
				return {
					type: 'ok',
					response: { type: 'batch', result: { step_results: stepResults, step_errors: stepErrors } }
				};
			}
			return { type: 'ok', response: { type: 'close' } };
		});

	const server: Server = createServer((req: IncomingMessage, res: ServerResponse) => {
		const chunks: Buffer[] = [];
		req.on('data', (c: Buffer) => chunks.push(c));
		req.on('end', () => {
			const path = (req.url ?? '/').split('?')[0];
			if (!path.endsWith('/pipeline')) {
				// Protocol probe: refuse so the client falls back to v2 JSON.
				res.writeHead(404);
				res.end();
				return;
			}
			counters.roundTrips += 1;
			const body = JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}') as { requests?: Json[] };
			const results = handle(body.requests ?? []);
			res.writeHead(200, { 'Content-Type': 'application/json' });
			res.end(JSON.stringify({ baton: null, base_url: null, results }));
		});
	});
	await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
	const address = server.address();
	const port = typeof address === 'object' && address ? address.port : 0;

	return {
		url: `http://127.0.0.1:${port}`,
		env: { TURSO_DATABASE_URL: `http://127.0.0.1:${port}`, TURSO_AUTH_TOKEN: 'test' },
		counters,
		statements,
		reset() {
			counters.statements = 0;
			counters.roundTrips = 0;
			counters.rowsRead = 0;
			counters.bytesRead = 0;
			statements.length = 0;
		},
		async close() {
			await new Promise<void>((resolve) => server.close(() => resolve()));
			db.close();
		},
		db
	};
}