export type SyncState = {
	google?: {
		[userEmail: string]: {
			historyId: string;
		};
	};
	microsoft?: {
		[userEmail: string]: {
			deltaTokens: { [folderId: string]: string };
		};
	};
	imap?: {
		[mailboxPath: string]: {
			maxUid: number;
		};
	};
	lastSyncTimestamp?: string;
	statusMessage?: string;
};

export type IngestionProvider =
	| 'google_workspace'
	| 'microsoft_365'
	| 'generic_imap'
	| 'pst_import'
	| 'eml_import'
	| 'mbox_import'
	| 'smtp_journaling';

export type IngestionStatus =
	| 'active'
	| 'paused'
	| 'error'
	| 'pending_auth'
	| 'syncing'
	| 'importing'
	| 'auth_success'
	| 'imported'
	| 'partially_active'; // For sources with merged children where some are active and others are not

export interface BaseIngestionCredentials {
	type: IngestionProvider;
}

export interface EMLImportCredentials extends BaseIngestionCredentials {
	type: 'eml_import';
	uploadedFileName?: string;
	uploadedFilePath?: string;
	localFilePath?: string;
}

export interface MboxImportCredentials extends BaseIngestionCredentials {
	type: 'mbox_import';
	uploadedFileName?: string;
	uploadedFilePath?: string;
	uploadedFiles?: Array<{
		fileName: string;
		filePath: string;
		relativePath?: string;
	}>;
	localFilePath?: string;
}

export interface SmtpJournalingCredentials extends BaseIngestionCredentials {
	type: 'smtp_journaling';
	/** The ID of the journaling_sources row that owns this ingestion source */
	journalingSourceId: string;
}

// Discriminated union for all possible credential types
export type IngestionCredentials =
	| EMLImportCredentials
	| MboxImportCredentials
	| SmtpJournalingCredentials;

export interface IngestionSource {
	id: string;
	name: string;
	provider: IngestionProvider;
	status: IngestionStatus;
	createdAt: Date;
	updatedAt: Date;
	credentials: IngestionCredentials;
	lastSyncStartedAt?: Date | null;
	lastSyncFinishedAt?: Date | null;
	lastSyncStatusMessage?: string | null;
	syncState?: SyncState | null;
	/** The ID of the root ingestion source this child is merged into.
	 *  Null or undefined when this source is a standalone root. */
	mergedIntoId?: string | null;
}

/**
 * Represents an ingestion source with sensitive credential information removed.
 * This type is safe to use in client-side applications or API responses
 * where exposing credentials would be a security risk.
 */
export type SafeIngestionSource = Omit<IngestionSource, 'credentials'>;

export interface CreateIngestionSourceDto {
	name: string;
	provider: IngestionProvider;
	providerConfig: Record<string, any>;
	/** Merge this new source into an existing root source's group. */
	mergedIntoId?: string;
}

export interface UpdateIngestionSourceDto {
	name?: string;
	provider?: IngestionProvider;
	status?: IngestionStatus;
	providerConfig?: Record<string, any>;
	lastSyncStartedAt?: Date;
	lastSyncFinishedAt?: Date;
	lastSyncStatusMessage?: string;
	syncState?: SyncState;
	/** Set or clear the merge parent. Use null to unmerge. */
	mergedIntoId?: string | null;
}

export interface IContinuousSyncJob {
	ingestionSourceId: string;
}

export interface IInitialImportJob {
	ingestionSourceId: string;
}

export interface IProcessMailboxJob {
	ingestionSourceId: string;
	userEmail: string;
	/** ID of the SyncSession tracking this sync cycle's progress */
	sessionId: string;
}

export type MailboxUser = {
	id: string;
	primaryEmail: string;
	displayName: string;
};

export type ProcessMailboxError = {
	error: boolean;
	message: string;
};
