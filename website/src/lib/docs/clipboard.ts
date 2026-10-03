// Clipboard write with a legacy fallback. `navigator.clipboard.writeText`
// is undefined or rejects on some mobile browsers (notably Firefox for
// Android), so fall back to a hidden textarea + execCommand, which works
// in those environments. Resolves true when the text was copied.
export async function copyText(text: string): Promise<boolean> {
	try {
		if (navigator.clipboard?.writeText) {
			await navigator.clipboard.writeText(text);
			return true;
		}
	} catch {
		// fall through to the legacy path below
	}
	try {
		const area = document.createElement('textarea');
		area.value = text;
		area.setAttribute('readonly', '');
		area.style.position = 'fixed';
		area.style.top = '-9999px';
		area.style.opacity = '0';
		document.body.appendChild(area);
		area.select();
		area.setSelectionRange(0, area.value.length);
		const ok = document.execCommand('copy');
		area.remove();
		return ok;
	} catch {
		return false;
	}
}
