const MONTHS = [
	'January',
	'February',
	'March',
	'April',
	'May',
	'June',
	'July',
	'August',
	'September',
	'October',
	'November',
	'December'
];

function ordinal(day: number): string {
	if (day >= 11 && day <= 13) return `${day}th`;
	switch (day % 10) {
		case 1:
			return `${day}st`;
		case 2:
			return `${day}nd`;
		case 3:
			return `${day}rd`;
		default:
			return `${day}th`;
	}
}

export function humanDate(ts: number, nowSec?: number): string {
	const now = nowSec ?? Math.floor(Date.now() / 1000);
	const diff = now - ts;
	if (diff < 60) return 'just now';
	if (diff < 3600) {
		const m = Math.floor(diff / 60);
		return m === 1 ? '1 minute ago' : `${m} minutes ago`;
	}
	if (diff < 86400) {
		const h = Math.floor(diff / 3600);
		return h === 1 ? '1 hour ago' : `${h} hours ago`;
	}
	if (Math.floor(diff / 86400) < 30) {
		const d = Math.floor(diff / 86400);
		return d === 1 ? '1 day ago' : `${d} days ago`;
	}
	const dt = new Date(ts * 1000);
	return `${ordinal(dt.getUTCDate())} ${MONTHS[dt.getUTCMonth()]} ${dt.getUTCFullYear()}`;
}
