import { app } from '../config/app';
import { logger } from '../config/logger';
import type { UpdateCheckResult, UpdateCommit } from '@open-archiver/types';

const GITHUB_API = 'https://api.github.com';

/**
 * Compares the commit stamped into this build (OA_GIT_SHA) against the tracked
 * branch on GitHub to tell the user whether an update is available. It never
 * applies anything — updates are applied on the host via `update-local.sh`.
 */
export class UpdateService {
	public async checkForUpdates(): Promise<UpdateCheckResult> {
		const currentSha = app.gitSha;
		const repo = app.updateRepo;
		const branch = app.updateBranch;

		const base: Omit<UpdateCheckResult, 'status'> = {
			currentSha,
			latestSha: null,
			updateAvailable: false,
			behindBy: 0,
			commits: [],
			compareUrl: null,
			checkedAt: new Date().toISOString(),
			updateCommand: app.updateCommand,
		};

		// Resolve the branch tip first — this works even if our own commit is unknown.
		const branchRes = await this.gh(`/repos/${repo}/commits/${branch}`);
		if (!branchRes.ok) {
			return {
				...base,
				status: 'error',
				message: `GitHub API returned ${branchRes.status} while resolving ${repo}@${branch}.`,
			};
		}
		const latestSha: string = (await branchRes.json()).sha;
		base.latestSha = latestSha;

		// No commit stamp (image built without GIT_SHA) — can't diff.
		if (!currentSha || currentSha === 'unknown') {
			return {
				...base,
				status: 'unknown',
				message: "This build wasn't stamped with a commit, so update status can't be determined.",
			};
		}

		if (currentSha === latestSha) {
			return { ...base, status: 'up_to_date' };
		}

		// Compare deployed...latest. `ahead_by` = commits the branch tip is ahead of us.
		const cmpRes = await this.gh(`/repos/${repo}/compare/${currentSha}...${latestSha}`);
		if (!cmpRes.ok) {
			// 404 typically means the deployed commit isn't on the remote (local-only commits).
			return {
				...base,
				status: 'unknown',
				message:
					"The deployed commit isn't on the remote, so how far behind can't be computed.",
			};
		}
		const cmp = await cmpRes.json();
		const behindBy: number = cmp.status === 'ahead' || cmp.status === 'diverged' ? cmp.ahead_by : 0;
		const commits: UpdateCommit[] = Array.isArray(cmp.commits)
			? cmp.commits.map((c: any) => ({
					sha: c.sha,
					message: String(c.commit?.message || '').split('\n')[0],
				}))
			: [];

		return {
			...base,
			updateAvailable: behindBy > 0,
			behindBy,
			commits,
			compareUrl: cmp.html_url ?? null,
			status: behindBy > 0 ? 'behind' : 'up_to_date',
		};
	}

	private gh(path: string): Promise<Response> {
		return fetch(`${GITHUB_API}${path}`, {
			headers: {
				Accept: 'application/vnd.github+json',
				'User-Agent': 'OpenArchiver-UpdateCheck',
			},
		}).catch((error) => {
			logger.error({ error, path }, 'GitHub update-check request failed');
			throw error;
		});
	}
}
