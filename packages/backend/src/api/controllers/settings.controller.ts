import type { Request, Response } from 'express';
import { SettingsService } from '../../services/SettingsService';
import { UserService } from '../../services/UserService';
import { UpdateService } from '../../services/UpdateService';
import { SearchService } from '../../services/SearchService';
import { logger } from '../../config/logger';

const settingsService = new SettingsService();
const userService = new UserService();
const updateService = new UpdateService();
const searchService = new SearchService();

export const getSystemSettings = async (req: Request, res: Response) => {
	try {
		const settings = await settingsService.getSystemSettings();
		res.status(200).json(settings);
	} catch (error) {
		// A more specific error could be logged here
		res.status(500).json({ message: req.t('settings.failedToRetrieve') });
	}
};

export const updateSystemSettings = async (req: Request, res: Response) => {
	try {
		// Basic validation can be performed here if necessary
		if (!req.user || !req.user.sub) {
			return res.status(401).json({ message: 'Unauthorized' });
		}
		const actor = await userService.findById(req.user.sub);
		if (!actor) {
			return res.status(401).json({ message: 'Unauthorized' });
		}
		const updatedSettings = await settingsService.updateSystemSettings(
			req.body,
			actor,
			req.ip || 'unknown'
		);
		res.status(200).json(updatedSettings);
	} catch (error) {
		// A more specific error could be logged here
		res.status(500).json({ message: req.t('settings.failedToUpdate') });
	}
};

/**
 * Wipes and rebuilds the full-text search index from the archive (re-parses
 * every stored email through the indexing queue). Used after migrations and
 * as a recovery tool.
 */
export const rebuildSearchIndex = async (_req: Request, res: Response) => {
	try {
		const result = await searchService.rebuildIndex();
		res.status(202).json(result);
	} catch (error) {
		logger.error({ error }, 'Search index rebuild failed');
		res.status(500).json({ message: 'Failed to start the search index rebuild.' });
	}
};

export const checkForUpdates = async (_req: Request, res: Response) => {
	try {
		const result = await updateService.checkForUpdates();
		res.status(200).json(result);
	} catch (error) {
		logger.error({ error }, 'Update check failed');
		res.status(502).json({ message: 'Failed to reach GitHub to check for updates.' });
	}
};
