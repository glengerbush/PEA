import { db } from '../database';
import * as schema from '../database/schema';
import { eq, sql } from 'drizzle-orm';
import { hash, compare } from 'bcryptjs';
import type { User } from '@open-archiver/types';
import { AuditService } from './AuditService';

export class UserService {
	private static auditService = new AuditService();

	public toPublicUser(user: typeof schema.users.$inferSelect): User {
		return {
			id: user.id,
			email: user.email,
			first_name: user.first_name,
			last_name: user.last_name,
			createdAt: user.createdAt,
		};
	}

	/**
	 * Finds a user by their email address.
	 * @param email The email address of the user to find.
	 * @returns The user object if found, otherwise null.
	 */
	public async findByEmail(email: string): Promise<typeof schema.users.$inferSelect | null> {
		const user = await db.query.users.findFirst({
			where: eq(schema.users.email, email),
		});
		return user || null;
	}

	/**
	 * Finds a user by their ID.
	 * @param id The ID of the user to find.
	 * @returns The user object if found, otherwise null.
	 */
	public async findById(id: string): Promise<User | null> {
		const user = await db.query.users.findFirst({
			where: eq(schema.users.id, id),
		});
		if (!user) return null;

		return this.toPublicUser(user);
	}

	public async updateUser(
		id: string,
		userDetails: Partial<Pick<User, 'email' | 'first_name' | 'last_name'>>,
		actor: User,
		actorIp: string
	): Promise<User | null> {
		const updatedUser = await db
			.update(schema.users)
			.set(userDetails)
			.where(eq(schema.users.id, id))
			.returning();

		await UserService.auditService.createAuditLog({
			actorIdentifier: actor.id,
			actionType: 'UPDATE',
			targetType: 'User',
			targetId: id,
			actorIp,
			details: {
				fields: Object.keys(userDetails),
			},
		});

		return updatedUser[0] ? this.toPublicUser(updatedUser[0]) : null;
	}

	public async updatePassword(
		id: string,
		currentPassword: string,
		newPassword: string,
		actor: User,
		actorIp: string
	): Promise<void> {
		const user = await db.query.users.findFirst({
			where: eq(schema.users.id, id),
		});

		if (!user || !user.password) {
			throw new Error('User not found');
		}

		const isPasswordValid = await compare(currentPassword, user.password);

		if (!isPasswordValid) {
			throw new Error('Invalid current password');
		}

		const hashedPassword = await hash(newPassword, 10);

		await db
			.update(schema.users)
			.set({ password: hashedPassword })
			.where(eq(schema.users.id, id));

		await UserService.auditService.createAuditLog({
			actorIdentifier: actor.id,
			actionType: 'UPDATE',
			targetType: 'User',
			targetId: id,
			actorIp,
			details: {
				field: 'password',
			},
		});
	}

	/**
	 * Creates the owner user in the database.
	 *
	 * Caution ⚠️: This action can only be allowed in the initial setup
	 *
	 * @param userDetails The details of the user to create.
	 * @param isSetup Is this an initial setup?
	 * @returns The newly created user object.
	 */
	public async createAdminUser(
		userDetails: Pick<User, 'email' | 'first_name' | 'last_name'> & { password?: string },
		isSetup: boolean
	): Promise<typeof schema.users.$inferSelect> {
		if (!isSetup) {
			throw Error('This operation is only allowed upon initial setup.');
		}
		const { email, first_name, last_name, password } = userDetails;
		const userCountResult = await db
			.select({ count: sql<number>`count(*)` })
			.from(schema.users);
		const isFirstUser = Number(userCountResult[0].count) === 0;
		if (!isFirstUser) {
			throw Error('This operation is only allowed upon initial setup.');
		}
		const hashedPassword = password ? await hash(password, 10) : undefined;

		const newUser = await db
			.insert(schema.users)
			.values({
				email,
				first_name,
				last_name,
				password: hashedPassword,
			})
			.returning();

		await UserService.auditService.createAuditLog({
			actorIdentifier: 'SYSTEM',
			actionType: 'SETUP',
			targetType: 'User',
			targetId: newUser[0].id,
			actorIp: '::1', // System action
			details: {
				setupAdminEmail: newUser[0].email,
			},
		});

		return newUser[0];
	}
}
