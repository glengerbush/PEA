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
	| 'importSource'
	| 'sourcePath'
	| 'tags';
export type ArchiveSortField = 'sentAt' | 'archivedAt' | 'sender' | 'subject' | 'sizeBytes';
export type SortDirection = 'asc' | 'desc';
export type SearchFieldMatch = 'all' | 'any';
export type MailboxSearchMode = 'simple' | 'advanced';
export type MailboxAttachmentFilter = 'any' | 'true' | 'false';

export interface MailboxSearchIntent {
	mode: MailboxSearchMode;
	q: string;
	fields: ArchiveSearchField[];
	senderQuery: string;
	recipientsQuery: string;
	subjectQuery: string;
	bodyQuery: string;
	fieldMatch: SearchFieldMatch;
	ingestionSourceId: string;
	hasAttachments: MailboxAttachmentFilter;
	attachmentExt: string;
	tags: string;
	page: number;
	limit: number;
	sort: ArchiveSortField;
	direction: SortDirection;
	matchingStrategy: MatchingStrategy;
	from: string;
}

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
