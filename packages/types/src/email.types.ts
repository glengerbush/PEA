// Define the structure of the document to be indexed for full-text search
export interface EmailDocument {
	id: string; // The unique ID of the email
	importSource: string;
	from: string;
	senderName: string;
	to: string[];
	cc: string[];
	bcc: string[];
	subject: string;
	body: string;
	attachments: {
		filename: string;
		content: string; // Extracted text from the attachment
	}[];
	timestamp: number;
	archivedAt: number;
	ingestionSourceId: string;
	threadId: string | null;
	messageIdHeader: string | null;
	hasAttachments: boolean;
	sourcePath: string | null;
	sourceLabels: string[];
	tags: string[];
	sizeBytes: number;
}
