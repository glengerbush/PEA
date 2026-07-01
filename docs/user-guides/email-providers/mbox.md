# Mbox Import

Mbox is a common format for storing email messages. This guide walks you through importing mbox files into OpenArchiver.

## 1. Exporting from Your Email Client

Most email clients that support mbox exports will allow you to export a folder of emails as a single `.mbox` file. Here are the general steps:

- **Mozilla Thunderbird**: Right-click on a folder, select **ImportExportTools NG**, and then choose **Export folder**.
- **Gmail**: You can use Google Takeout to export your emails in mbox format.
- **Other Clients**: Refer to your email client's documentation for instructions on how to export emails to an mbox file.

## 2. Uploading to OpenArchiver

Once you have your `.mbox` file or a folder of `.mbox` files, you can import it through the web interface.

1.  Go to the **Imports** page.
2.  Click **Import Archive**.
3.  Select **Mbox** as the provider.
4.  **Choose Import Method:**
    - **Upload File:** Select one or more flat `.mbox` files, or use the Apple Mail folder picker for a `.mbox` package. Select a parent folder to add several packages at once, or use the picker repeatedly to append folders.
    - **Local Path:** Enter the path to one `.mbox` file, an Apple Mail `.mbox` package, or a folder containing multiple archives **inside the container**. Folder imports are scanned recursively, so subfolders are included.

    > **Note on Local Path:** When using Docker, the path must exist inside the OpenArchiver container. A host path such as `/home/you/mail` is not visible unless it is mounted into the container. The upload option does not require a mount.
    >
    > - **Recommended:** Place your mbox file or mbox folder in a `temp` folder inside your configured storage directory (`STORAGE_LOCAL_ROOT_PATH`). This path is already mounted. For example, if your storage path is `/data`, put files under `/data/temp/mbox-import/` and enter `/data/temp/mbox-import` as the path.
    > - **Alternative:** Mount a separate volume in `docker-compose.yml` (e.g., `- /host/path:/container/path`) and use the container path.

## 3. Folder Structure

OpenArchiver will attempt to preserve the original folder structure of your emails. This is done by inspecting the following email headers:

- `X-Gmail-Labels`: Used by Gmail to store labels.
- `X-Folder`: A custom header used by some email clients like Thunderbird.

If neither of these headers is present, a single uploaded or local file is ingested into the root of the archive. For a local folder import, OpenArchiver uses each `.mbox` file's relative path as the fallback folder. For example, `/data/temp/mbox-import/Clients/acme.mbox` imports to `Clients/acme`.
