import { Request, Response } from 'express';
import { dashboardService } from '../../services/DashboardService';

class DashboardController {
	public async getStats(req: Request, res: Response) {
		const stats = await dashboardService.getStats();
		res.json(stats);
	}

	public async getIngestionHistory(req: Request, res: Response) {
		const history = await dashboardService.getIngestionHistory();
		res.json(history);
	}

	public async getIngestionSources(req: Request, res: Response) {
		const sources = await dashboardService.getIngestionSources();
		res.json(sources);
	}

	public async getRecentSyncs(req: Request, res: Response) {
		const syncs = await dashboardService.getRecentSyncs();
		res.json(syncs);
	}

	public async getIndexedInsights(req: Request, res: Response) {
		const insights = await dashboardService.getIndexedInsights();
		res.json(insights);
	}

	public async getRemoteContentIssues(req: Request, res: Response) {
		const page = Math.max(1, parseInt(String(req.query.page ?? '1'), 10) || 1);
		const limit = Math.min(100, Math.max(1, parseInt(String(req.query.limit ?? '25'), 10) || 25));
		const statusParam = String(req.query.status ?? 'all');
		const status = (['all', 'failed', 'partial'].includes(statusParam) ? statusParam : 'all') as
			| 'all'
			| 'failed'
			| 'partial';
		const sortParam = String(req.query.sort ?? 'date');
		const sort = (['date', 'subject', 'status'].includes(sortParam) ? sortParam : 'date') as
			| 'date'
			| 'subject'
			| 'status';
		const direction = req.query.direction === 'asc' ? 'asc' : 'desc';
		const result = await dashboardService.getRemoteContentIssuesPage({
			page,
			limit,
			status,
			sort,
			direction,
		});
		res.json(result);
	}
}

export const dashboardController = new DashboardController();
