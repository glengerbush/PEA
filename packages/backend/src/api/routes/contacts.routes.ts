import { Router } from 'express';
import { ContactsController } from '../controllers/contacts.controller';
import { requireAuth } from '../middleware/requireAuth';

export const createContactsRouter = (): Router => {
	const router = Router();
	const controller = new ContactsController();

	router.use(requireAuth());

	router.get('/map', controller.getContactMap);
	router.post('/import', controller.importContacts);

	return router;
};
