export function newRequestId(): string {
	return crypto.randomUUID();
}

const REQUEST_ID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

export function isRequestId(value: unknown): boolean {
	return typeof value === 'string' && REQUEST_ID_RE.test(value);
}
