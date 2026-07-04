export type AlertType = {
	type: 'success' | 'warning' | 'error';
	title: string;
	message: string;
	duration: number;
	show: boolean;
};

export type AlertItem = {
	id: number;
	type: AlertType['type'];
	title: string;
	message: string;
	duration: number;
};

let alerts = $state<AlertItem[]>([]);
let nextId = 0;

/**
 * Push a new alert onto the stack. Multiple alerts stack (and stay visible)
 * instead of replacing each other — e.g. approving several duplicate groups
 * in quick succession now shows one toast per action.
 */
export function setAlert(alert: AlertType) {
	// Callers that pass `show: false` (an explicit "clear") are a no-op with a
	// stack — dismissal is per-item now.
	if (alert.show === false) return;
	const id = ++nextId;
	alerts.push({
		id,
		type: alert.type,
		title: alert.title,
		message: alert.message,
		duration: alert.duration,
	});
}

export function dismissAlert(id: number) {
	const idx = alerts.findIndex((a) => a.id === id);
	if (idx !== -1) alerts.splice(idx, 1);
}

export function getAlerts(): AlertItem[] {
	return alerts;
}
