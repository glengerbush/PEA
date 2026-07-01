import { Router } from 'express';
import * as settingsController from '../controllers/settings.controller';
import { requireAuth } from '../middleware/requireAuth';
import { AuthService } from '../../services/AuthService';

export const createSettingsRouter = (authService: AuthService): Router => {
	const router = Router();

	/**
	 * @openapi
	 * /v1/settings/system:
	 *   get:
	 *     summary: Get system settings
	 *     description: >
	 *       Returns non-sensitive system settings such as language, timezone, and feature flags.
	 *       This endpoint is public — no authentication required. Sensitive settings are never exposed.
	 *     operationId: getSystemSettings
	 *     tags:
	 *       - Settings
	 *     responses:
	 *       '200':
	 *         description: Current system settings.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/SystemSettings'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 *   put:
	 *     summary: Update system settings
	 *     description: Updates system settings. Requires authentication.
	 *     operationId: updateSystemSettings
	 *     tags:
	 *       - Settings
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     requestBody:
	 *       required: true
	 *       content:
	 *         application/json:
	 *           schema:
	 *             $ref: '#/components/schemas/SystemSettings'
	 *     responses:
	 *       '200':
	 *         description: Updated system settings.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/SystemSettings'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '403':
	 *         $ref: '#/components/responses/Forbidden'
	 *       '500':
	 *         $ref: '#/components/responses/InternalServerError'
	 */
	// Public route to get non-sensitive settings. All end users need the settings data in the frontend.
	router.get('/system', settingsController.getSystemSettings);

	// Protected route to update settings
	router.put('/system', requireAuth(authService), settingsController.updateSystemSettings);

	// Protected route to check whether a newer build is available on the fork.
	router.get('/updates/check', requireAuth(authService), settingsController.checkForUpdates);

	return router;
};
