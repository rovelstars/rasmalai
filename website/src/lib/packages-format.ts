export const PACKAGE_FORMAT_NOTICE =
	'Formatter unavailable for this file; showing stored source as-is.';

export function isFormattablePackagePath(path: string): boolean {
	return path.endsWith('.rnx');
}

export interface PackageFormatOutcome {
	text: string;
	formatted: boolean;
	notice: string | null;
}

interface FormatPayload {
	ok: boolean;
	output?: unknown;
}

export function selectPackageText(
	raw: string,
	path: string,
	formatRaw: string | null
): PackageFormatOutcome {
	if (isFormattablePackagePath(path)) {
		try {
			const payload = JSON.parse(formatRaw ?? '') as FormatPayload;
			if (payload?.ok === true && typeof payload.output === 'string') {
				return { text: payload.output, formatted: true, notice: null };
			}
		} catch {
			void 0;
		}
		return { text: raw, formatted: false, notice: PACKAGE_FORMAT_NOTICE };
	}
	return { text: raw, formatted: false, notice: null };
}
