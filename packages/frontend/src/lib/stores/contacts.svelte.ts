import type { ContactMap } from '@open-archiver/types';

let contactMap = $state<ContactMap>({});

export function setContacts(map: ContactMap | undefined | null) {
	contactMap = map || {};
}

/** Resolve an imported contact's display name for an email address, if known. */
export function contactName(email: string | null | undefined): string | undefined {
	if (!email) return undefined;
	return contactMap[email.trim().toLowerCase()];
}
