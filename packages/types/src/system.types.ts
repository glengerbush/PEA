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
	clockFormat: '12h' | '24h';
}
