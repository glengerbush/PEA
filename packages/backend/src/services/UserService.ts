import { db } from '../database';
import * as schema from '../database/schema';
import { eq, sql } from 'drizzle-orm';
import { hash, compare } from 'bcryptjs';
import type { User } from '@open-archiver/types';

export class UserService {

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

	/**
	 * Returns the single local user used when authentication is disabled (local
	 * mode). Resolves the first existing user — which owns all archived content —
	 * and provisions a placeholder user if the database is empty so foreign-key
	 * constraints and ownership filters still have a valid identity.
	 *
	 * Used by the auth bypass; remove/ignore when restoring real authentication.
	 */
	public async getOrCreateLocalUser(): Promise<typeof schema.users.$inferSelect> {
		const existing = await db.query.users.findFirst();
		if (existing) return existing;

		const [created] = await db
			.insert(schema.users)
			.values({
				email: process.env.ADMIN_EMAIL || 'local@localhost',
				first_name: 'Local',
				last_name: 'User',
			})
			.returning();
		return created;
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


		return newUser[0];
	}
}
