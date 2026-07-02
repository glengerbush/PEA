import { Router } from 'express';
import * as userController from '../controllers/user.controller';
import { requireAuth } from '../middleware/requireAuth';

export const createUserRouter = (): Router => {
	const router = Router();

	router.use(requireAuth());

	/**
	 * @openapi
	 * /v1/users/profile:
	 *   get:
	 *     summary: Get current user profile
	 *     description: Returns the profile of the currently authenticated user.
	 *     operationId: getProfile
	 *     tags:
	 *       - Users
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     responses:
	 *       '200':
	 *         description: Current user's profile.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/User'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '404':
	 *         $ref: '#/components/responses/NotFound'
	 *   patch:
	 *     summary: Update current user profile
	 *     description: Updates the email, first name, or last name of the currently authenticated user. Disabled in demo mode.
	 *     operationId: updateProfile
	 *     tags:
	 *       - Users
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     requestBody:
	 *       required: true
	 *       content:
	 *         application/json:
	 *           schema:
	 *             type: object
	 *             properties:
	 *               email:
	 *                 type: string
	 *                 format: email
	 *               first_name:
	 *                 type: string
	 *               last_name:
	 *                 type: string
	 *     responses:
	 *       '200':
	 *         description: Updated user profile.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/User'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '403':
	 *         description: Disabled in demo mode.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/ErrorMessage'
	 */
	router.get('/profile', userController.getProfile);
	router.patch('/profile', userController.updateProfile);

	/**
	 * @openapi
	 * /v1/users/profile/password:
	 *   post:
	 *     summary: Update password
	 *     description: Updates the password of the currently authenticated user. The current password must be provided for verification. Disabled in demo mode.
	 *     operationId: updatePassword
	 *     tags:
	 *       - Users
	 *     security:
	 *       - bearerAuth: []
	 *       - apiKeyAuth: []
	 *     requestBody:
	 *       required: true
	 *       content:
	 *         application/json:
	 *           schema:
	 *             type: object
	 *             required:
	 *               - currentPassword
	 *               - newPassword
	 *             properties:
	 *               currentPassword:
	 *                 type: string
	 *                 format: password
	 *               newPassword:
	 *                 type: string
	 *                 format: password
	 *     responses:
	 *       '200':
	 *         description: Password updated successfully.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/MessageResponse'
	 *       '400':
	 *         description: Current password is incorrect.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/ErrorMessage'
	 *       '401':
	 *         $ref: '#/components/responses/Unauthorized'
	 *       '403':
	 *         description: Disabled in demo mode.
	 *         content:
	 *           application/json:
	 *             schema:
	 *               $ref: '#/components/schemas/ErrorMessage'
	 */
	router.post('/profile/password', userController.updatePassword);

	return router;
};
