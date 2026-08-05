import { describe, expect, it } from 'vitest';
import type { ApproveDuplicatesResult, DuplicateGroup } from '@pea/types';
import {
	approvalFeedback,
	approvalIntent,
	duplicateGroupsPageUrl,
	pageAfterGroupRemoval,
	refreshAfterGroupRemoval,
} from './duplicate-review';

const group = {
	groupKey: 'opaque-group-key',
	classification: 'exact',
	count: 2,
	keeperEmailId: 'default-keeper',
	emails: [{ id: 'default-keeper' }, { id: 'other-copy' }],
} as DuplicateGroup;

describe('duplicate review adapter', () => {
	it('sends only the selected keeper intent', () => {
		expect(approvalIntent(group, { 'opaque-group-key': 'other-copy' })).toEqual({
			groupKey: 'opaque-group-key',
			keeperEmailId: 'other-copy',
		});
	});

	it('clamps a depleted final page', () => {
		expect(pageAfterGroupRemoval({ totalGroups: 26, page: 2, limit: 25 }, 2)).toBe(1);
		expect(pageAfterGroupRemoval({ totalGroups: 51, page: 2, limit: 25 }, 1)).toBe(2);
	});

	it('refreshes the clamped page after a decision', async () => {
		const visits: unknown[] = [];
		await refreshAfterGroupRemoval(
			{ totalGroups: 26, page: 2, limit: 25 },
			2,
			'exact',
			async (...args) => {
				visits.push(args);
			}
		);
		expect(visits).toEqual([
			['/dashboard/duplicates', { invalidateAll: true, keepFocus: true, replaceState: true }],
		]);
	});

	it('preserves classification and target page in URLs', () => {
		expect(duplicateGroupsPageUrl('likely', 3)).toBe(
			'/dashboard/duplicates?classification=likely&exactPage=3'
		);
	});

	it('warns when stale decisions are rejected', () => {
		const result: ApproveDuplicatesResult = {
			approvedGroups: 1,
			deletedEmails: 1,
			keeperEmails: 1,
			rejectedGroups: [{ groupKey: 'stale', reason: 'unknown_group' }],
		};
		expect(approvalFeedback(result)).toEqual({
			type: 'warning',
			title: 'Some groups not deleted',
			message: '1 duplicate email deleted; 1 group rejected because review data changed',
		});
	});
});
