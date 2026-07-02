import { Router } from 'express';
import { IngestionController } from '../controllers/ingestion.controller';
import { requireAuth } from '../middleware/requireAuth';

export const createIngestionRouter = (
	ingestionController: IngestionController): Router => {
	const router = Router();

	// Secure all routes in this module
	router.use(requireAuth());

	/**
	 * @openapi
	 * /v1/ingestion-sources:
	 *   post:
	 *     summary: Create an ingestion source
	 *     description: Creates a new ingestion source and validates the connection. Returns the created source without credentials. Requires authentication.
	 *     operationId: createIngestionSource
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     requestBody:
	 *       required: true
	 *       content:
	 *         application/json:
	 *           schema:
	 *             $ref: '#/components/schemas/CreateIngestionSourceDto'
	 *     responses:
	 *       '201':
	 *         description: Ingestion source created successfully.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/SafeIngestionSource'
	 *       '400':
	 *         description: Invalid input or connection test failed.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/ErrorMessage'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *   get:
	 *     summary: List ingestion sources
	 *     description: Returns all ingestion sources accessible to the authenticated user. Credentials are excluded from the response. Requires authentication.
	 *     operationId: listIngestionSources
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     responses:
	 *       '200':
	 *         description: Array of ingestion sources.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               type: array
	 *               items:
	 *                 $ref: '#/components/schemas/SafeIngestionSource'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 */
	router.post('/', ingestionController.create);

	router.get('/', ingestionController.findAll);

	/**
	 * @openapi
	 * /v1/ingestion-sources/{id}:
	 *   get:
	 *     summary: Get an ingestion source
	 *     description: Returns a single ingestion source by ID. Credentials are excluded. Requires authentication.
	 *     operationId: getIngestionSourceById
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *     responses:
	 *       '200':
	 *         description: Ingestion source details.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/SafeIngestionSource'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 *   put:
	 *     summary: Update an ingestion source
	 *     description: Updates configuration for an existing ingestion source. Requires authentication.
	 *     operationId: updateIngestionSource
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *     requestBody:
	 *       required: true
	 *       content:
	 *         application/json:
	 *           schema:
	 *             $ref: '#/components/schemas/UpdateIngestionSourceDto'
	 *     responses:
	 *       '200':
	 *         description: Updated ingestion source.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/SafeIngestionSource'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 *   delete:
	 *     summary: Delete an ingestion source
	 *     description: Permanently deletes an ingestion source. Deletion must be enabled in system settings. Requires authentication.
	 *     operationId: deleteIngestionSource
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *     responses:
	 *       '204':
	 *         description: Ingestion source deleted. No content returned.
	 *       '400':
	 *         description: Deletion disabled or constraint error.
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
	router.get('/:id', ingestionController.findById);

	router.put('/:id', ingestionController.update);

	router.delete('/:id', ingestionController.delete);

	/**
	 * @openapi
	 * /v1/ingestion-sources/{id}/import:
	 *   post:
	 *     summary: Trigger initial import
	 *     description: Enqueues an initial import job for the ingestion source. This imports all historical emails. Requires authentication.
	 *     operationId: triggerInitialImport
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *     responses:
	 *       '202':
	 *         description: Initial import job accepted and queued.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/MessageResponse'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 */
	router.post('/:id/import', ingestionController.triggerInitialImport);

	/**
	 * @openapi
	 * /v1/ingestion-sources/{id}/pause:
	 *   post:
	 *     summary: Pause an ingestion source
	 *     description: Sets the ingestion source status to `paused`, stopping continuous sync. Requires authentication.
	 *     operationId: pauseIngestionSource
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *     responses:
	 *       '200':
	 *         description: Ingestion source paused. Returns the updated source.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/SafeIngestionSource'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 */
	router.post('/:id/pause', ingestionController.pause);

	/**
	 * @openapi
	 * /v1/ingestion-sources/{id}/sync:
	 *   post:
	 *     summary: Force sync
	 *     description: Triggers an out-of-schedule continuous sync for the ingestion source. Requires authentication.
	 *     operationId: triggerForceSync
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         schema:
	 *           type: string
	 *           example: "clx1y2z3a0000b4d2"
	 *     responses:
	 *       '202':
	 *         description: Force sync job accepted and queued.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/MessageResponse'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 */
	router.post('/:id/sync', ingestionController.triggerForceSync);

	/**
	 * @openapi
	 * /v1/ingestion-sources/{id}/unmerge:
	 *   post:
	 *     summary: Unmerge a child ingestion source
	 *     description: Detaches a child source from its merge group, making it a standalone root source. Requires authentication.
	 *     operationId: unmergeIngestionSource
	 *     tags:
	 *       - Ingestion
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     parameters:
	 *       - name: id
	 *         in: path
	 *         required: true
	 *         schema:
	 *           type: string
	 *     responses:
	 *       '200':
	 *         description: Source unmerged. Returns the updated source.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/SafeIngestionSource'
	 *       '400':
	 *         description: Source is not merged into another source.
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 */
	router.post('/:id/unmerge', ingestionController.unmerge);

	return router;
};
