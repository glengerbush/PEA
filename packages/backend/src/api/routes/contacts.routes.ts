import { Router } from 'express';
import { ContactsController } from '../controllers/contacts.controller';
import { requireAuth } from '../middleware/requireAuth';
import { AuthService } from '../../services/AuthService';

export const createContactsRouter = (authService: AuthService): Router => {
	const router = Router();
	const controller = new ContactsController();

	router.use(requireAuth(authService));

	router.get('/map', controller.getContactMap);
	router.post('/import', controller.importContacts);

	return router;
};
