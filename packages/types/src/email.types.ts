/**
 * What a stored email timestamp actually represents, so the UI can label it
 * honestly instead of showing every date as an exact send time.
 *  - `sent`: a valid, timezoned Date header — the real send instant.
 *  - `sent_zone_unknown`: a Date header with no timezone; the wall-clock is
 *    shown verbatim (never shifted to the viewer's zone) with a note.
 *  - `received`: no Date header, so the earliest Received time is used and
 *    labeled as a received (not sent) time.
 *  - `unknown`: no timestamp anywhere; shown as "date unknown".
 */
export type DateKind = 'sent' | 'sent_zone_unknown' | 'received' | 'unknown';

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
	/** What `timestamp` is (sent / zone-unknown / received / unknown). */
	timestampKind: DateKind;
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
