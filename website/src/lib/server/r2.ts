export interface R2BucketLike {
	head(key: string): Promise<unknown | null>;
	get(key: string): Promise<{ arrayBuffer(): Promise<ArrayBuffer> } | null>;
	put(key: string, value: Uint8Array, options?: Record<string, unknown>): Promise<unknown>;
	delete(key: string): Promise<void>;
}

export function r2Bucket(env: Record<string, unknown>): R2BucketLike | null {
	const b = (env as Record<string, unknown>)['CHUNKS'];
	if (!b || typeof b !== 'object') return null;
	const bucket = b as Partial<R2BucketLike>;
	if (typeof bucket.head !== 'function' || typeof bucket.get !== 'function' || typeof bucket.put !== 'function') {
		return null;
	}
	return bucket as R2BucketLike;
}

export function chunkKey(hash: string): string {
	const h = hash.toLowerCase();
	return `chunks/${h.slice(0, 2)}/${h.slice(2, 4)}/${h}`;
}

export async function r2Has(bucket: R2BucketLike, hash: string): Promise<boolean> {
	return (await bucket.head(chunkKey(hash))) !== null;
}

export async function r2PutIfMissing(bucket: R2BucketLike, hash: string, bytes: Uint8Array): Promise<boolean> {
	if (await r2Has(bucket, hash)) return false;
	await bucket.put(chunkKey(hash), bytes, {
		httpMetadata: { contentType: 'application/octet-stream' }
	});
	return true;
}

export async function r2Get(bucket: R2BucketLike, hash: string): Promise<Uint8Array | null> {
	const obj = await bucket.get(chunkKey(hash));
	if (!obj) return null;
	return new Uint8Array(await obj.arrayBuffer());
}

export async function r2Delete(bucket: R2BucketLike, hash: string): Promise<void> {
	await bucket.delete(chunkKey(hash));
}
