import type { SystemSettings } from '@pea/types';

/**
 * Global date/time display preferences, sourced from system settings. Set once
 * from the root layout via {@link setDateTimePrefs}; read by the format helpers
 * below. Because these are `$state`, any `{formatDateTime(x)}` in a template
 * re-renders automatically when the preferences change (e.g. after saving
 * settings), without a page reload.
 */
const prefs = $state<{ timeZone: string | undefined; hour12: boolean }>({
	timeZone: undefined,
	hour12: true
});

export function setDateTimePrefs(
	settings: Pick<SystemSettings, 'timeZone' | 'clockFormat'> | null | undefined
): void {
	prefs.timeZone = settings?.timeZone || undefined;
	prefs.hour12 = settings?.clockFormat ? settings.clockFormat === '12h' : true;
}

type DateInput = string | number | Date;

function toDate(value: DateInput): Date | null {
	const date = value instanceof Date ? value : new Date(value);
	return Number.isNaN(date.getTime()) ? null : date;
}

/** Date + time, honoring the configured time zone and 12/24-hour clock. */
export function formatDateTime(value: DateInput, options: Intl.DateTimeFormatOptions = {}): string {
	const date = toDate(value);
	if (!date) return '';
	return date.toLocaleString(undefined, {
		timeZone: prefs.timeZone,
		hour12: prefs.hour12,
		...options
	});
}

/** Date only, honoring the configured time zone. */
export function formatDate(value: DateInput, options: Intl.DateTimeFormatOptions = {}): string {
	const date = toDate(value);
	if (!date) return '';
	return date.toLocaleDateString(undefined, {
		timeZone: prefs.timeZone,
		...options
	});
}

