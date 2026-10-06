export const GC_GRACE_DAYS = 30;
export const GC_DEFAULT_LIMIT = 200;
export const GC_MAX_LIMIT = 1000;

export function gcCutoffSec(nowSec: number, graceDays: number = GC_GRACE_DAYS): number {
	return Math.floor(nowSec) - Math.floor(graceDays) * 86400;
}

export function parseGcLimit(value: unknown): number {
	const n = typeof value === 'string' ? Number(value) : typeof value === 'number' ? value : NaN;
	if (!Number.isFinite(n)) return GC_DEFAULT_LIMIT;
	return Math.max(1, Math.min(GC_MAX_LIMIT, Math.floor(n)));
}
