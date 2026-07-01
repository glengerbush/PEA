import { StorageConfig } from '@open-archiver/types';
import 'dotenv/config';

const storageType = process.env.STORAGE_TYPE;
const encryptionKey = process.env.STORAGE_ENCRYPTION_KEY;
const openArchiverFolderName = 'open-archiver';
let storageConfig: StorageConfig;

if (encryptionKey && !/^[a-fA-F0-9]{64}$/.test(encryptionKey)) {
	throw new Error('STORAGE_ENCRYPTION_KEY must be a 64-character hex string (32 bytes)');
}

if (storageType === 'local') {
	if (!process.env.STORAGE_LOCAL_ROOT_PATH) {
		throw new Error('STORAGE_LOCAL_ROOT_PATH is not defined in the environment variables');
	}
	storageConfig = {
		type: 'local',
		rootPath: process.env.STORAGE_LOCAL_ROOT_PATH,
		openArchiverFolderName: openArchiverFolderName,
		encryptionKey: encryptionKey,
	};
} else {
	throw new Error(`Invalid STORAGE_TYPE: ${storageType} (only 'local' is supported)`);
}

export const storage = storageConfig;
