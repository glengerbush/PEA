import type { EmailDocument } from './email.types';

export type MatchingStrategy = 'last' | 'all';
export type ArchiveSearchField =
	| 'subject'
	| 'body'
	| 'from'
	| 'senderName'
	| 'to'
	| 'cc'
	| 'bcc'
	| 'attachments.filename'
	| 'attachments.content'
	| 'userEmail'
	| 'sourcePath'
	| 'tags';
export type ArchiveSortField = 'sentAt' | 'archivedAt' | 'sender' | 'subject' | 'sizeBytes';
export type SortDirection = 'asc' | 'desc';

export interface SearchHit extends EmailDocument {
	_matchesPosition?: {
		[key: string]: { start: number; length: number; indices?: number[] }[];
	};
	_formatted?: Partial<EmailDocument>;
}

export interface SearchResult {
	hits: SearchHit[];
	total: number;
	page: number;
	limit: number;
	totalPages: number;
	processingTimeMs: number;
}
