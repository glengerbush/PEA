export type ContactImportFormat = 'csv' | 'vcf';

export interface ImportContactsResult {
	parsed: number;
	imported: number;
	updated: number;
	skipped: number;
}

/** Lowercased email address → display name. */
export type ContactMap = Record<string, string>;
