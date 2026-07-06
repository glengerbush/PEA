export type SupportedLanguage =
	| 'en' // English
	| 'es' // Spanish
	| 'fr' // French
	| 'de' // German
	| 'it' // Italian
	| 'pt' // Portuguese
	| 'nl' // Dutch
	| 'ja' // Japanese
	| 'et' // Estonian
	| 'el' // Greek
	| 'bg'; // Bulgarian

export type Theme = 'light' | 'dark' | 'system';

/** How times are displayed. */
export type ClockFormat = '12h' | '24h';

/**
 * How dates are displayed. 'system' follows the viewer's locale; the rest force a
 * day/month/year ordering regardless of locale.
 */
export type DateFormat = 'system' | 'mdy' | 'dmy' | 'ymd' | 'long';

export interface SystemSettings {
	/** The default display language for the application UI. */
	language: SupportedLanguage;

	/** The default color theme for the application. */
	theme: Theme;

	/**
	 * IANA time zone used to display dates and times (e.g. "America/New_York").
	 * Null means use the viewer's local time zone.
	 */
	timeZone: string | null;

	/** Whether to display times in 12-hour (AM/PM) or 24-hour format. */
	clockFormat: ClockFormat;

	/** How dates are displayed (locale-automatic, or a fixed day/month/year order). */
	dateFormat: DateFormat;

	/**
	 * When true, the desktop app checks for updates at launch and prompts before
	 * installing. When false, updates are only checked when the user asks. Has no
	 * effect on the standalone engine (which never self-updates).
	 */
	autoCheckUpdates: boolean;
}
