import { Request, Response } from 'express';
import { ContactsService } from '../../services/ContactsService';
import type { ContactImportFormat } from '@open-archiver/types';

export class ContactsController {
	public importContacts = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const format = req.body?.format as ContactImportFormat;
			const content = req.body?.content;
			if ((format !== 'csv' && format !== 'vcf') || typeof content !== 'string') {
				return res.status(400).json({ message: 'A "format" (csv|vcf) and "content" are required' });
			}

			const result = await ContactsService.importContacts(format, content);
			return res.status(200).json(result);
		} catch (error) {
			console.error('Import contacts error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(500).json({ message });
		}
	};

	public getContactMap = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}
			const map = await ContactsService.getContactMap();
			return res.status(200).json(map);
		} catch (error) {
			console.error('Get contact map error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(500).json({ message });
		}
	};
}
