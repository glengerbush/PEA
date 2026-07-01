import { Router } from 'express';
import { ArchivedEmailController } from '../controllers/archived-email.controller';
import { requireAuth } from '../middleware/requireAuth';
import { AuthService } from '../../services/AuthService';

export const createArchivedEmailRouter = (
	archivedEmailController: ArchivedEmailController,
	authService: AuthService
): Router => {
	const router = Router();

	// Secure all routes in this module
	router.use(requireAuth(authService));

	router.get('/', archivedEmailController.queryArchivedEmails);

	router.get('/facets', archivedEmailController.listFilterFacets);

	router.post('/bulk/tags', archivedEmailController.updateArchivedEmailTags);
	router.post('/bulk/delete', archivedEmailController.bulkDeleteArchivedEmails);

	router.get('/duplicates/exact', archivedEmailController.listExactDuplicateGroups);

	router.post('/duplicates/exact/approve', archivedEmailController.approveExactDuplicateGroups);

	router.get('/duplicates/fuzzy', archivedEmailController.listFuzzyDuplicateGroups);

	router.post('/duplicates/fuzzy/scan', archivedEmailController.scanFuzzyDuplicateGroups);

	router.post('/duplicates/fuzzy/approve', archivedEmailController.approveFuzzyDuplicateGroups);

	router.post('/duplicates/fuzzy/ignore', archivedEmailController.ignoreFuzzyDuplicateGroups);

	router.get('/:id/preview', archivedEmailController.getRemoteContentPreview);

	router.get('/:id/remote-assets', archivedEmailController.listRemoteContentAssets);

	router.post('/:id/remote-content/archive', archivedEmailController.enqueueRemoteContentArchive);

	router.get('/:id/remote-assets/:assetId', archivedEmailController.getRemoteContentAsset);

	/**
	 * @openapi
	 * /v1/archived-emails/ingestion-source/{ingestionSourceId}:
	 *   get:
	 *     summary: List archived emails for an ingestion source
	 *     description: Returns a paginated list of archived emails belonging to the specified ingestion source. Requires authentication.
	 *     operationId: getArchivedEmails
	 *     tags:
	 *       - Archived Emails
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: ingestionSourceId
	 *         in: path
	 *         required: true
	 *         description: The ID of the ingestion source to retrieve emails for.
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *       - name: page
	 *         in: query
	 *         required: false
	 *         description: Page number for pagination.
	 *         schema:
	 *           type: integer
	 *           default: 1
	 *           example: 1
	 *       - name: limit
	 *         in: query
	 *         required: false
	 *         description: Number of items per page.
	 *         schema:
	 *           type: integer
	 *           default: 10
	 *           example: 10
	 *     responses:
	 *       '200':
	 *         description: Paginated list of archived emails.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/PaginatedArchivedEmails'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 */
	router.get('/ingestion-source/:ingestionSourceId', archivedEmailController.getArchivedEmails);

	/**
	 * @openapi
	 * /v1/archived-emails/{id}:
	 *   get:
	 *     summary: Get a single archived email
	 *     description: Retrieves the full details of a single archived email by ID, including attachments and thread. Requires authentication.
	 *     operationId: getArchivedEmailById
	 *     tags:
	 *       - Archived Emails
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         description: The ID of the archived email.
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *     responses:
	 *       '200':
	 *         description: Archived email details.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/ArchivedEmail'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 *   delete:
	 *     summary: Delete an archived email
	 *     description: Permanently deletes an archived email by ID. Deletion must be enabled in system settings and the email must not be on legal hold. Requires authentication.
	 *     operationId: deleteArchivedEmail
	 *     tags:
	 *       - Archived Emails
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         description: The ID of the archived email to delete.
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *     responses:
	 *       '204':
	 *         description: Email deleted successfully. No content returned.
	 *       '400':
	 *         description: Deletion is disabled in system settings, or the email is blocked by a retention policy / legal hold.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/ErrorMessage'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 */
	router.get('/:id', archivedEmailController.getArchivedEmailById);

	router.delete('/:id', archivedEmailController.deleteArchivedEmail);

	return router;
};
