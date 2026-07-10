/**
 * Represents a single recipient of an email.
 */
export interface Recipient {
	name?: string;
	email: string;
	/** Which header this recipient came from, for separate To/Cc/Bcc display. */
	kind?: 'to' | 'cc' | 'bcc';
}

/**
 * Represents a single attachment of an email.
 */
export interface Attachment {
	id: string;
	filename: string;
	mimeType: string | null;
	sizeBytes: number;
	storagePath: string;
	/** Content-Description header, when the sender included one. */
	contentDescription?: string | null;
	/** RFC 2183 file timestamps from Content-Disposition, as sent. */
	originalCreatedAt?: string | null;
	originalModifiedAt?: string | null;
}

export interface ThreadEmail {
	id: string; //the archivedemail id
	subject: string | null;
	sentAt: Date;
	senderEmail: string;
	hasAttachments: boolean;
}

export interface BulkDeleteArchivedEmailsResult {
	requestedCount: number;
	deletedCount: number;
	deletedIds: string[];
	/** Emails that could not be deleted (e.g. legal hold / retention policy). */
	failed: { id: string; reason: string }[];
}

export interface UpdatedArchivedEmailTags {
	id: string;
	tags: string[];
}

export interface UpdateArchivedEmailTagsResult {
	requestedCount: number;
	updatedCount: number;
	addedTags: string[];
	removedTags: string[];
	emails: UpdatedArchivedEmailTags[];
}

export type ExactDuplicateReason =
	| 'message_id'
	| 'storage_hash'
	| 'attachment_hash_set'
	| 'sender_recipients_sent'
	| 'message_body';

/** Group counts per reason (plus `all`), independent of the active filter, so
 *  each filter pill can show its own count. */
export type ExactDuplicateReasonCounts = Record<ExactDuplicateReason | 'all', number>;

export interface ExactDuplicateEmail {
	id: string;
	subject: string | null;
	senderName: string | null;
	senderEmail: string;
	importSource: string;
	sentAt: Date;
	archivedAt: Date;
	hasAttachments: boolean;
	sourcePath: string | null;
	messageIdHeader: string | null;
	storageHashSha256: string;
}

export interface ExactDuplicateGroup {
	groupKey: string;
	/** Primary (highest-priority) reason this cluster was detected. */
	reason: ExactDuplicateReason;
	/** All reasons that link this cluster (a cluster can match several). */
	reasons: ExactDuplicateReason[];
	fingerprint: string;
	count: number;
	keeperEmailId: string;
	emails: ExactDuplicateEmail[];
}

export interface ExactDuplicateGroupsResult {
	groups: ExactDuplicateGroup[];
	/** Group count for the active reason filter. */
	totalGroups: number;
	/** Per-reason group counts (and `all`), for the filter pills. */
	reasonCounts: ExactDuplicateReasonCounts;
	page: number;
	limit: number;
}

export interface ApproveExactDuplicateGroupDto {
	groupKey: string;
	keeperEmailId: string;
	duplicateEmailIds: string[];
}

export interface ApproveAllExactDuplicatesDto {
	/** Restrict to clusters matching this reason; omit to approve every cluster. */
	reason?: ExactDuplicateReason;
}

export interface ApproveExactDuplicatesResult {
	approvedGroups: number;
	/** Duplicate copies permanently deleted (the keeper of each group is preserved). */
	deletedEmails: number;
	keeperEmails: number;
}

export interface IgnoreExactDuplicateGroupsResult {
	ignoredGroups: number;
}

export type RemoteContentStatus =
	'not_started' | 'pending' | 'archived' | 'partial' | 'failed' | 'skipped';

export type RemoteContentAssetStatus = 'pending' | 'archived' | 'blocked' | 'failed';

export interface RemoteContentPreview {
	emailId: string;
	html: string;
	status: RemoteContentStatus;
	remoteUrlCount: number;
	archivedAssetCount: number;
	blockedAssetCount: number;
	failedAssetCount: number;
}

/** Slim, client-facing view of a remote-content asset for the detail-page list. */
export interface RemoteContentAssetSummary {
	id: string;
	originalUrl: string;
	contentType: string | null;
	sizeBytes: number | null;
	/** archived | failed | blocked (pending assets are not returned). */
	status: RemoteContentAssetStatus;
	/** Why the asset failed or was blocked (often includes an HTTP status / error code). */
	failureReason: string | null;
	/** True when archived and the content type is safe to render inline (e.g. images). */
	previewable: boolean;
}

/**
 * Represents a single archived email.
 */
export interface ArchivedEmail {
	id: string;
	ingestionSourceId: string;
	threadId: string | null;
	importSource: string;
	messageIdHeader: string | null;
	providerMessageId: string | null;
	sentAt: Date;
	subject: string | null;
	senderName: string | null;
	senderEmail: string;
	recipients: Recipient[];
	storagePath: string;
	storageHashSha256: string;
	sizeBytes: number;
	hasAttachments: boolean;
	archivedAt: Date;
	sourcePath: string | null;
	duplicateSubjectHash: string | null;
	duplicateBodyHash: string | null;
	duplicateRecipientFingerprint: string | null;
	duplicateAttachmentFingerprint: string | null;
	remoteContentStatus: RemoteContentStatus;
	remoteContentAssetCount: number;
	remoteContentArchivedAt: Date | null;
	attachments?: Attachment[];
	thread?: ThreadEmail[];
	tags: string[] | null;
}
