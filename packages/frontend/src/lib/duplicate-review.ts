import type {
	ApproveDuplicateGroupDto,
	ApproveDuplicatesResult,
	DuplicateClassification,
	DuplicateGroup,
	DuplicateGroupsPageState,
} from '@pea/types';

export function approvalIntent(
	group: DuplicateGroup,
	keeperOverrides: Readonly<Record<string, string>>
): ApproveDuplicateGroupDto | null {
	const keeperEmailId = keeperOverrides[group.groupKey] || group.keeperEmailId;
	if (!keeperEmailId) return null;
	return { groupKey: group.groupKey, keeperEmailId };
}

export function pageAfterGroupRemoval(
	result: DuplicateGroupsPageState,
	removedGroups: number
): number {
	const remaining = Math.max(0, result.totalGroups - removedGroups);
	const totalPages = Math.max(1, Math.ceil(remaining / Math.max(1, result.limit)));
	return Math.min(result.page, totalPages);
}

export function duplicateGroupsPageUrl(
	classification: DuplicateClassification,
	page: number
): string {
	const params = new URLSearchParams();
	if (classification !== 'exact') params.set('classification', classification);
	if (page > 1) params.set('exactPage', String(page));
	const query = params.toString();
	return `/dashboard/duplicates${query ? `?${query}` : ''}`;
}

export async function refreshAfterGroupRemoval(
	result: DuplicateGroupsPageState,
	removedGroups: number,
	classification: DuplicateClassification,
	navigate: (
		url: string,
		options: { invalidateAll: true; keepFocus: true; replaceState: true }
	) => Promise<void>
): Promise<void> {
	const page = pageAfterGroupRemoval(result, removedGroups);
	await navigate(duplicateGroupsPageUrl(classification, page), {
		invalidateAll: true,
		keepFocus: true,
		replaceState: true,
	});
}

export function approvalFeedback(result: ApproveDuplicatesResult): {
	type: 'success' | 'warning';
	title: string;
	message: string;
} {
	const deleted = `${result.deletedEmails} duplicate email${result.deletedEmails === 1 ? '' : 's'} deleted`;
	const rejected = result.rejectedGroups.length;
	if (rejected === 0) {
		return { type: 'success', title: 'Duplicates deleted', message: deleted };
	}
	return {
		type: 'warning',
		title: result.approvedGroups > 0 ? 'Some groups not deleted' : 'No duplicates deleted',
		message: `${deleted}; ${rejected} group${rejected === 1 ? '' : 's'} rejected because review data changed`,
	};
}
