export const RESOLVE_RATE_LIMIT = 60;
export const RESOLVE_RATE_WINDOW_MS = 60_000;
export const LIST_RATE_LIMIT = 120;
export const LIST_RATE_WINDOW_MS = 60_000;

const MAX_TRACKED_KEYS = 4096;

interface Window {
	start: number;
	count: number;
}

const windows = new Map<string, Window>();

export type RateLimitVerdict = { ok: true } | { ok: false; retryAfterSec: number };

export function resetRateLimits(): void {
	windows.clear();
}

export function rateLimitSize(): number {
	return windows.size;
}

export function checkRateLimit(
	key: string,
	limit: number,
	windowMs: number,
	now: number = Date.now()
): RateLimitVerdict {
	const w = windows.get(key);
	if (w && now - w.start < windowMs) {
		if (w.count >= limit) {
			return { ok: false, retryAfterSec: Math.max(1, Math.ceil((w.start + windowMs - now) / 1000)) };
		}
		w.count += 1;
		return { ok: true };
	}
	if (!w && windows.size >= MAX_TRACKED_KEYS) {
		const oldest = windows.keys().next();
		if (!oldest.done) windows.delete(oldest.value);
	}
	windows.set(key, { start: now, count: 1 });
	return { ok: true };
}
