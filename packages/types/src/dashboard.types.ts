export interface DashboardStats {
	totalEmailsArchived: number;
	totalStorageUsed: number;
	failedIngestionsLast7Days: number;
	/** Emails whose remote content failed to fetch entirely. */
	remoteContentFailed: number;
	/** Emails whose remote content was only partially fetched. */
	remoteContentPartial: number;
}

export interface IngestionHistory {
	history: {
		date: string;
		count: number;
	}[];
}

export interface IngestionSourceStats {
	id: string;
	name: string;
	provider: string;
	status: string;
	storageUsed: number;
}

export interface RecentSync {
	id: string;
	sourceName: string;
	startTime: string;
	duration: number;
	emailsProcessed: number;
	status: string;
}

export interface TopSender {
	sender: string;
	count: number;
}

export interface IndexedInsights {
	topSenders: TopSender[];
}

/** A single remote asset (image/stylesheet/etc.) that failed or was blocked. */
export interface RemoteContentIssueAsset {
	url: string;
	/** 'failed' | 'blocked' */
	status: string;
	/** Human-readable reason the fetch failed/was blocked, if recorded. */
	reason: string | null;
}

/** An email whose remote-content archiving failed or only partially succeeded. */
export interface RemoteContentIssue {
	emailId: string;
	subject: string;
	sender: string;
	status: 'failed' | 'partial';
	/** When the email was archived (ISO), for the date column / sorting. */
	archivedAt: string;
	assets: RemoteContentIssueAsset[];
}

/** A page of remote-content issues, for the paginated issues table. */
export interface RemoteContentIssuesResult {
	items: RemoteContentIssue[];
	total: number;
	page: number;
	limit: number;
}
