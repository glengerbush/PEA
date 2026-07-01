/**
 * Represents a single recipient of an email.
 */
export interface Recipient {
	name?: string;
	email: string;
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
}

export interface ThreadEmail {
	id: string; //the archivedemail id
	subject: string | null;
	sentAt: Date;
	senderEmail: string;
	hasAttachments: boolean;
}

export interface BulkDeleteArchivedEmailsDto {
	emailIds: string[];
}

export interface BulkDeleteArchivedEmailsResult {
	requestedCount: number;
	deletedCount: number;
	deletedIds: string[];
	/** Emails that could not be deleted (e.g. legal hold / retention policy). */
	failed: { id: string; reason: string }[];
}

export interface UpdateArchivedEmailTagsDto {
	emailIds: string[];
	addTags?: string[];
	removeTags?: string[];
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
	| 'sender_recipients_sent';

export interface ExactDuplicateEmail {
	id: string;
	subject: string | null;
	senderName: string | null;
	senderEmail: string;
	userEmail: string;
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
	totalGroups: number;
	page: number;
	limit: number;
}

export interface ApproveExactDuplicateGroupDto {
	groupKey: string;
	keeperEmailId: string;
	duplicateEmailIds: string[];
}

export interface ApproveExactDuplicatesDto {
	groups: ApproveExactDuplicateGroupDto[];
}

export interface ApproveExactDuplicatesResult {
	approvedGroups: number;
	/** Duplicate copies permanently deleted (the keeper of each group is preserved). */
	deletedEmails: number;
	keeperEmails: number;
}

export interface FuzzyDuplicateSignals {
	senderEmail: string;
	subjectHash: string;
	matchingBodyHash: boolean;
	matchingRecipients: boolean;
	matchingAttachments: boolean;
	sentSpreadHours: number | null;
}

export interface FuzzyDuplicateEmail extends ExactDuplicateEmail {
	suggestedKeeper: boolean;
}

export interface FuzzyDuplicateGroup {
	id: string;
	groupKey: string;
	status: 'pending' | 'approved' | 'ignored';
	score: number;
	signals: FuzzyDuplicateSignals;
	createdAt: Date;
	updatedAt: Date;
	keeperEmailId: string;
	emails: FuzzyDuplicateEmail[];
}

export interface FuzzyDuplicateGroupsResult {
	groups: FuzzyDuplicateGroup[];
	totalGroups: number;
	page: number;
	limit: number;
}

export interface ScanFuzzyDuplicatesDto {
	batchSize?: number;
}

export interface ScanFuzzyDuplicatesResult {
	jobId: string | number;
	batchSize: number;
}

export interface FuzzyDuplicateScanResult {
	scannedGroups: number;
	insertedGroups: number;
	linkedEmails: number;
}

export interface ApproveFuzzyDuplicateGroupDto {
	groupId: string;
	keeperEmailId: string;
	duplicateEmailIds: string[];
}

export interface ApproveFuzzyDuplicatesDto {
	groups: ApproveFuzzyDuplicateGroupDto[];
}

export interface ApproveFuzzyDuplicatesResult {
	approvedGroups: number;
	/** Duplicate copies permanently deleted (the keeper of each group is preserved). */
	deletedEmails: number;
	keeperEmails: number;
}

export interface IgnoreFuzzyDuplicateGroupsDto {
	groupIds: string[];
}

export interface IgnoreFuzzyDuplicateGroupsResult {
	ignoredGroups: number;
}

export type RemoteContentStatus =
	| 'not_started'
	| 'pending'
	| 'archived'
	| 'partial'
	| 'failed'
	| 'skipped';

export type RemoteContentAssetStatus = 'pending' | 'archived' | 'blocked' | 'failed';

export interface RemoteContentAsset {
	id: string;
	emailId: string;
	originalUrl: string;
	finalUrl: string | null;
	urlHash: string;
	status: RemoteContentAssetStatus;
	contentType: string | null;
	sizeBytes: number | null;
	contentHashSha256: string | null;
	storagePath: string | null;
	failureReason: string | null;
	createdAt: Date;
	updatedAt: Date;
}

export interface RemoteContentPreview {
	emailId: string;
	html: string;
	status: RemoteContentStatus;
	remoteUrlCount: number;
	archivedAssetCount: number;
	blockedAssetCount: number;
	failedAssetCount: number;
}

export interface ArchiveRemoteContentResult {
	jobId: string | number;
	emailIds: string[];
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
	userEmail: string;
	messageIdHeader: string | null;
	sentAt: Date;
	subject: string | null;
	senderName: string | null;
	senderEmail: string;
	recipients: Recipient[];
	storagePath: string;
	storageHashSha256: string;
	sizeBytes: number;
	isIndexed: boolean;
	hasAttachments: boolean;
	archivedAt: Date;
	sourcePath: string | null;
	sourceLabels: string[] | null;
	duplicateSubjectHash: string | null;
	duplicateFuzzyGroupKey: string | null;
	duplicateBodyHash: string | null;
	duplicateRecipientFingerprint: string | null;
	duplicateAttachmentFingerprint: string | null;
	remoteContentStatus: RemoteContentStatus;
	remoteContentAssetCount: number;
	remoteContentArchivedAt: Date | null;
	attachments?: Attachment[];
	raw?: Buffer;
	thread?: ThreadEmail[];
	path: string | null;
	tags: string[] | null;
}

/**
 * Represents a paginated list of archived emails.
 */
export interface PaginatedArchivedEmails {
	items: ArchivedEmail[];
	total: number;
	page: number;
	limit: number;
}
