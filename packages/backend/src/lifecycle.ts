/**
 * Shutdown hook registry. Modules that own external resources (e.g. the
 * embedded Postgres child) register a hook; bootstrap runs them, last-in
 * first-out, after the app itself has drained.
 *
 * Deliberately dependency-free: it must be importable before any config/env
 * capture happens.
 */

type ShutdownHook = { label: string; run: () => Promise<void> };

const hooks: ShutdownHook[] = [];

export const registerShutdownHook = (label: string, run: () => Promise<void>): void => {
	hooks.push({ label, run });
};

export const runShutdownHooks = async (
	onError?: (label: string, error: unknown) => void
): Promise<void> => {
	for (const hook of [...hooks].reverse()) {
		try {
			await hook.run();
		} catch (error) {
			onError?.(hook.label, error);
		}
	}
	hooks.length = 0;
};
