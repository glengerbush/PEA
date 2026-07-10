import type { SystemSettings, DateFormat } from '@pea/types';

/**
 * Global date/time display preferences, sourced from system settings. Set once
 * from the root layout via {@link setDateTimePrefs}, and again after the settings
 * form saves; read by the format helpers below. Because these are `$state`, any
 * `{formatDateTime(x)}` in a template re-renders automatically when the
 * preferences change, without a page reload.
 */
const prefs = $state<{ timeZone: string | undefined; hour12: boolean; dateFormat: DateFormat }>({
	timeZone: undefined,
	hour12: true,
	dateFormat: 'system',
});

export function setDateTimePrefs(
	settings: Pick<SystemSettings, 'timeZone' | 'clockFormat' | 'dateFormat'> | null | undefined
): void {
	prefs.timeZone = settings?.timeZone || undefined;
	prefs.hour12 = settings?.clockFormat ? settings.clockFormat === '12h' : true;
	prefs.dateFormat = settings?.dateFormat ?? 'system';
}

type DateInput = string | number | Date;

function toDate(value: DateInput): Date | null {
	const date = value instanceof Date ? value : new Date(value);
	return Number.isNaN(date.getTime()) ? null : date;
}

/** Time-zone-correct year/month/day as zero-padded strings. */
function ymdParts(date: Date): { y: string; m: string; d: string } {
	const parts = new Intl.DateTimeFormat('en-US', {
		timeZone: prefs.timeZone,
		year: 'numeric',
		month: '2-digit',
		day: '2-digit',
	}).formatToParts(date);
	const get = (t: string) => parts.find((p) => p.type === t)?.value ?? '';
	return { y: get('year'), m: get('month'), d: get('day') };
}

/**
 * Applies the fixed date-format preference, or null when it's 'system' (in which
 * case the caller should fall back to locale formatting). Numeric orderings are
 * assembled from time-zone-correct parts so the order is guaranteed.
 */
function formatFixedDate(date: Date): string | null {
	if (prefs.dateFormat === 'system') return null;
	if (prefs.dateFormat === 'long') {
		return date.toLocaleDateString(undefined, {
			timeZone: prefs.timeZone,
			year: 'numeric',
			month: 'long',
			day: 'numeric',
		});
	}
	const { y, m, d } = ymdParts(date);
	switch (prefs.dateFormat) {
		case 'mdy':
			return `${m}/${d}/${y}`;
		case 'dmy':
			return `${d}/${m}/${y}`;
		case 'ymd':
			return `${y}-${m}-${d}`;
		default:
			return null;
	}
}

/** Date + time, honoring the configured time zone, date format, and 12/24h clock. */
export function formatDateTime(value: DateInput, options: Intl.DateTimeFormatOptions = {}): string {
	const date = toDate(value);
	if (!date) return '';
	// Explicit caller options fall back to locale formatting so they're honored.
	const fixed = Object.keys(options).length === 0 ? formatFixedDate(date) : null;
	if (fixed !== null) {
		const time = date.toLocaleTimeString(undefined, {
			timeZone: prefs.timeZone,
			hour12: prefs.hour12,
			hour: 'numeric',
			minute: '2-digit',
		});
		return `${fixed}, ${time}`;
	}
	return date.toLocaleString(undefined, {
		timeZone: prefs.timeZone,
		hour12: prefs.hour12,
		...options,
	});
}

/** Date only, honoring the configured time zone and date format. */
export function formatDate(value: DateInput, options: Intl.DateTimeFormatOptions = {}): string {
	const date = toDate(value);
	if (!date) return '';
	const fixed = Object.keys(options).length === 0 ? formatFixedDate(date) : null;
	if (fixed !== null) return fixed;
	return date.toLocaleDateString(undefined, { timeZone: prefs.timeZone, ...options });
}
