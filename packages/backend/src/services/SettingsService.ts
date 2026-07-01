import { db } from '../database';
import { systemSettings } from '../database/schema/system-settings';
import type { SystemSettings, User } from '@open-archiver/types';

const DEFAULT_SETTINGS: SystemSettings = {
	language: 'en',
	theme: 'system',
	timeZone: null,
	clockFormat: '12h',
};

export class SettingsService {
	/**
	 * Retrieves the current system settings.
	 * If no settings exist, it initializes and returns the default settings.
	 * @returns The system settings.
	 */
	public async getSystemSettings(): Promise<SystemSettings> {
		const settings = await db.select().from(systemSettings).limit(1);

		if (settings.length === 0) {
			return this.createDefaultSystemSettings();
		}

		// Merge with defaults so settings added after this record was created
		// (e.g. timeZone / clockFormat) still resolve to a value.
		return { ...DEFAULT_SETTINGS, ...settings[0].config };
	}

	/**
	 * Updates the system settings by merging the new configuration with the existing one.
	 * @param newConfig - A partial object of the new settings configuration.
	 * @returns The updated system settings.
	 */
	public async updateSystemSettings(
		newConfig: Partial<SystemSettings>,
		actor: User,
		actorIp: string
	): Promise<SystemSettings> {
		const currentConfig = await this.getSystemSettings();
		const mergedConfig = { ...currentConfig, ...newConfig };

		// Since getSettings ensures a record always exists, we can directly update.
		const [result] = await db.update(systemSettings).set({ config: mergedConfig }).returning();

		return result.config;
	}

	/**
	 * Creates and saves the default system settings.
	 * This is called internally when no settings are found.
	 * @returns The newly created default settings.
	 */
	private async createDefaultSystemSettings(): Promise<SystemSettings> {
		const [result] = await db
			.insert(systemSettings)
			.values({ config: DEFAULT_SETTINGS })
			.returning();
		return result.config;
	}
}
