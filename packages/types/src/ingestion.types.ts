export type IngestionProvider = 'eml_import' | 'mbox_import';

export type IngestionStatus =
	| 'active'
	| 'paused'
	| 'error'
	| 'pending' // created; validating the source before import
	| 'ready' // validated; the queue is about to start the import
	| 'importing'
	| 'imported'
	| 'partially_active'; // For sources with merged children where some are active and others are not

export interface BaseProviderConfig {
	type: IngestionProvider;
}

export interface EMLImportProviderConfig extends BaseProviderConfig {
	type: 'eml_import';
	localFilePath?: string;
}

export interface MboxImportProviderConfig extends BaseProviderConfig {
	type: 'mbox_import';
	localFilePath?: string;
}

// Discriminated union for all possible provider-config shapes
export type IngestionProviderConfig = EMLImportProviderConfig | MboxImportProviderConfig;

export interface IngestionSource {
	id: string;
	name: string;
	provider: IngestionProvider;
	status: IngestionStatus;
	createdAt: Date;
	updatedAt: Date;
	providerConfig: IngestionProviderConfig;
	lastImportStartedAt?: Date | null;
	lastImportFinishedAt?: Date | null;
	lastImportStatusMessage?: string | null;
	/** The ID of the root ingestion source this child is merged into.
	 *  Null or undefined when this source is a standalone root. */
	mergedIntoId?: string | null;
}

/**
 * Represents an ingestion source with its provider config omitted. For the
 * local mbox/eml importers the config is just a file path (no secrets), but
 * the safe shape keeps the API response and client models lean and uniform.
 */
export type SafeIngestionSource = Omit<IngestionSource, 'providerConfig'>;

export interface CreateIngestionSourceDto {
	name: string;
	provider: IngestionProvider;
	providerConfig: Record<string, any>;
	/** Merge this new source into an existing root source's group. */
	mergedIntoId?: string;
}
