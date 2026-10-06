// Cost guard, and it stays. R2 bills real money and the registry writes
// bytes on other people's behalf, so every upload passes this check first.
// Donations may cover the bill someday, but no funding lasts forever, so
// this file is permanent: set QUOTA_GUARD=off to disable it while funding
// is stable, and flip it back on whenever costs need watching again.
// Sampling is pity-style: a flat 0.1% base rate that ramps quadratically
// once usage passes 75% of budget and goes certain at 99%. No polling, no
// metadata tables — the usage number comes from SUM(size_bytes) over the
// chunk mapping we already keep, plus bytes written since the last sample.
const BASE_RATE = 0.001;
const SOFT_PITY = 0.75;
const HARD_PITY = 0.99;

export function quotaGuardEnabled(env: Record<string, string | undefined>): boolean {
	return (env['QUOTA_GUARD'] ?? '').trim().toLowerCase() !== 'off';
}

export function sampleProbability(u: number): number {
	if (!(u >= 0)) return BASE_RATE;
	if (u >= HARD_PITY) return 1;
	if (u <= SOFT_PITY) return BASE_RATE;
	const t = (u - SOFT_PITY) / (HARD_PITY - SOFT_PITY);
	return Math.min(1, BASE_RATE + (1 - BASE_RATE) * t * t);
}

export interface QuotaState {
	sampled: number | null;
	localBytes: number;
}

export function freshQuotaState(): QuotaState {
	return { sampled: null, localBytes: 0 };
}

export function effectiveUsage(state: QuotaState, budgetBytes: number): number {
	if (!(budgetBytes > 0)) return 0;
	return ((state.sampled ?? 0) + state.localBytes) / budgetBytes;
}

export function shouldSample(state: QuotaState, budgetBytes: number, roll: number): boolean {
	if (state.sampled === null) return true;
	return roll < sampleProbability(effectiveUsage(state, budgetBytes));
}

export function quotaBudgetBytes(env: Record<string, string | undefined>): number {
	const raw = Number(env['QUOTA_R2_BYTES'] ?? '');
	if (Number.isFinite(raw) && raw > 0) return Math.floor(raw);
	return 10 * 1024 * 1024 * 1024;
}
