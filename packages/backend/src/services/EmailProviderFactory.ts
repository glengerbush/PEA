import type {
	IngestionSource,
	EMLImportCredentials,
	MboxImportCredentials,
	EmailObject,
	SyncState,
	MailboxUser,
} from '@open-archiver/types';
import { EMLConnector } from './ingestion-connectors/EMLConnector';
import { MboxConnector } from './ingestion-connectors/MboxConnector';

// Define a common interface for all connectors
export interface IEmailConnector {
	testConnection(): Promise<boolean>;
	fetchEmails(
		userEmail: string,
		syncState?: SyncState | null,
		checkDuplicate?: (messageId: string) => Promise<boolean>
	): AsyncGenerator<EmailObject | null>;
	getUpdatedSyncState(userEmail?: string): SyncState;
	listAllUsers(): AsyncGenerator<MailboxUser>;
}

export class EmailProviderFactory {
	static createConnector(source: IngestionSource): IEmailConnector {
		// Credentials are now decrypted by the IngestionService before being passed around
		const credentials = source.credentials;

		switch (source.provider) {
			case 'eml_import':
				return new EMLConnector(credentials as EMLImportCredentials);
			case 'mbox_import':
				return new MboxConnector(credentials as MboxImportCredentials);
			default:
				throw new Error(`Unsupported provider: ${source.provider}`);
		}
	}
}
