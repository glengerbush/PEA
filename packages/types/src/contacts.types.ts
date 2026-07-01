export interface Contact {
	id: string;
	email: string;
	displayName: string;
	source: string | null;
	createdAt: Date;
	updatedAt: Date;
}

export type ContactImportFormat = 'csv' | 'vcf';

export interface ImportContactsDto {
	format: ContactImportFormat;
	/** Raw file contents (CSV text or vCard text). */
	content: string;
}

export interface ImportContactsResult {
	parsed: number;
	imported: number;
	updated: number;
	skipped: number;
}

/** Lowercased email address → display name. */
export type ContactMap = Record<string, string>;
