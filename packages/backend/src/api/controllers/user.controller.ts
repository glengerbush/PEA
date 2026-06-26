import { Request, Response } from 'express';
import { UserService } from '../../services/UserService';
import { config } from '../../config';

const userService = new UserService();

export const getProfile = async (req: Request, res: Response) => {
	if (!req.user || !req.user.sub) {
		return res.status(401).json({ message: 'Unauthorized' });
	}
	const user = await userService.findById(req.user.sub);
	if (!user) {
		return res.status(404).json({ message: req.t('user.notFound') });
	}
	res.json(user);
};

export const updateProfile = async (req: Request, res: Response) => {
	if (config.app.isDemo) {
		return res.status(403).json({ message: req.t('errors.demoMode') });
	}
	const { email, first_name, last_name } = req.body;
	if (!req.user || !req.user.sub) {
		return res.status(401).json({ message: 'Unauthorized' });
	}
	const actor = await userService.findById(req.user.sub);
	if (!actor) {
		return res.status(401).json({ message: 'Unauthorized' });
	}
	const updatedUser = await userService.updateUser(
		req.user.sub,
		{ email, first_name, last_name },
		actor,
		req.ip || 'unknown'
	);
	res.json(updatedUser);
};

export const updatePassword = async (req: Request, res: Response) => {
	if (config.app.isDemo) {
		return res.status(403).json({ message: req.t('errors.demoMode') });
	}
	const { currentPassword, newPassword } = req.body;
	if (!req.user || !req.user.sub) {
		return res.status(401).json({ message: 'Unauthorized' });
	}
	const actor = await userService.findById(req.user.sub);
	if (!actor) {
		return res.status(401).json({ message: 'Unauthorized' });
	}

	try {
		await userService.updatePassword(
			req.user.sub,
			currentPassword,
			newPassword,
			actor,
			req.ip || 'unknown'
		);
		res.status(200).json({ message: 'Password updated successfully' });
	} catch (e: any) {
		if (e.message === 'Invalid current password') {
			return res.status(400).json({ message: e.message });
		}
		throw e;
	}
};
