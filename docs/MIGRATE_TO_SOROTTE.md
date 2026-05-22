# Migrate To Sorotte

Sorotte is a hard rename. New builds do not read old `syncplay-*` package names, `SYNCPLAY_*` runtime environment variables, old appdata folders, old update manifests, or old release artifacts.

Use the script below, or copy the files manually before starting Sorotte.

## Scripted Copy

The migration script copies by default and leaves the old files in place:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/migrate-syncplay-to-sorotte.ps1
```

Useful options:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/migrate-syncplay-to-sorotte.ps1 -DryRun
powershell -ExecutionPolicy Bypass -File scripts/migrate-syncplay-to-sorotte.ps1 -Force
powershell -ExecutionPolicy Bypass -File scripts/migrate-syncplay-to-sorotte.ps1 -Move
powershell -ExecutionPolicy Bypass -File scripts/migrate-syncplay-to-sorotte.ps1 -OldRoot C:\Users\you\AppData\Roaming -NewRoot C:\Users\you\AppData\Roaming\Sorotte
```

The script migrates old file-based config, GUI state files, and managed stream-helper tools. It does not make Sorotte read old paths at runtime.

## Config And Appdata

Sorotte writes `sorotte.ini` under an app-scoped folder:

| Platform | New path |
| --- | --- |
| Windows | `%APPDATA%\Sorotte\sorotte.ini` |
| Linux/BSD | `${XDG_CONFIG_HOME:-$HOME/.config}/sorotte/sorotte.ini` |
| macOS | `$HOME/Library/Application Support/Sorotte/sorotte.ini` |

Users can move Sorotte's appdata-style files into a custom folder. New installs use install-folder `sorotte.ini` as a locator, so Sorotte can find the custom root before reading the active settings. On first startup, Sorotte creates install-folder `sorotte.ini` if it is missing and points it at the platform default Sorotte folder; if the file already exists, startup leaves it untouched. Environment and CLI overrides take precedence:

1. `--config-path <file>` or `--config-root <dir>`
2. `SOROTTE_CLIENT_CONFIG_PATH=<file>`
3. `SOROTTE_CLIENT_CONFIG_ROOT=<dir>`
4. install-folder `sorotte.ini`
5. legacy GUI-saved custom root pointer
6. platform default appdata root

When changing the root from the GUI, Sorotte saves the current configuration into the new root, updates install-folder `sorotte.ini`, and copies known GUI state, cache, stream-helper, and update-staging files on a best-effort basis. It does not delete or move the old folder. If the chosen root is the install folder itself, install-folder `sorotte.ini` is both the locator and normal settings file, and stores `configRoot = .` instead of an absolute path so the folder remains portable.

Manual copy targets:

| Old item | New item |
| --- | --- |
| `%APPDATA%\syncplay.ini` | `%APPDATA%\Sorotte\sorotte.ini` |
| `%APPDATA%\.syncplay` | `%APPDATA%\Sorotte\sorotte.ini` |
| `${XDG_CONFIG_HOME:-$HOME/.config}/syncplay.ini` | `${XDG_CONFIG_HOME:-$HOME/.config}/sorotte/sorotte.ini` |
| `${XDG_CONFIG_HOME:-$HOME/.config}/.syncplay` | `${XDG_CONFIG_HOME:-$HOME/.config}/sorotte/sorotte.ini` |
| `Syncplay\MainWindow.ini` and related GUI state files | Sorotte app folder, same filename |
| `Syncplay\tools\stream-helper` | Sorotte app folder `tools\stream-helper` |

If both old config files exist, choose the one you have actually been using. The script prefers `syncplay.ini`, then `.syncplay`.

## Environment Variables

Rename runtime variables and remove the old names:

| Old | New |
| --- | --- |
| `SYNCPLAY_CLIENT_*` | `SOROTTE_CLIENT_*` |
| `SYNCPLAY_GUI_*` | `SOROTTE_GUI_*` |
| `SYNCPLAY_SERVER_*` | `SOROTTE_SERVER_*` |
| `SYNCPLAY_PASSWORD` | `SOROTTE_PASSWORD` |
| `SYNCPLAY_SALT` | `SOROTTE_SALT` |
| `SYNCPLAY_MPV_IPC_PATH` | `SOROTTE_MPV_IPC_PATH` |
| `SYNCPLAY_CLIENT_MPV_IPC_PATH` | `SOROTTE_CLIENT_MPV_IPC_PATH` |

Compatibility/oracle variables that explicitly refer to the Python Syncplay reference implementation keep their names, for example `SYNCPLAY_LEGACY_ROOT`.

## Commands And Packages

Use the new binaries and package names:

| Old | New |
| --- | --- |
| `syncplay-cli` | `sorotte-cli` |
| `syncplay-gui` | `sorotte-gui` |
| `syncplay-gui-updater` | `sorotte-gui-updater` |
| `syncplay-server` | `sorotte-server` |
| `syncplay-gui-<version>-windows-x86_64.zip` | `sorotte-gui-<version>-windows-x86_64.zip` |
| `syncplay-server-<version>-windows-x86_64.zip` | `sorotte-server-<version>-windows-x86_64.zip` |
| `syncplay-server-<version>-linux-x86_64.tar.gz` | `sorotte-server-<version>-linux-x86_64.tar.gz` |

## Docker And GHCR

Update image names and persistent volume names explicitly:

```powershell
docker pull ghcr.io/ropbet-radbyt/sorotte-server:latest
docker run --rm -p 8999:8999/tcp -v ${PWD}/sorotte-data:/data ghcr.io/ropbet-radbyt/sorotte-server:latest
```

If you used a volume or host directory named `syncplay-data`, either rename it or mount it deliberately as the new Sorotte server data volume. Do the same for TLS directories and scripts.

## GitHub Repositories

Use the new repositories:

| Purpose | Repository |
| --- | --- |
| Source | `ropbet-radbyt/sorotte` |
| GUI releases | `ropbet-radbyt/sorotte` |
| Server image | `ghcr.io/ropbet-radbyt/sorotte-server` |

Local git remote update:

```powershell
git remote set-url origin https://github.com/ropbet-radbyt/sorotte.git
```

Release automation now publishes GUI downloads to GitHub Releases in `ropbet-radbyt/sorotte`. Do not publish new artifacts under old `syncplay-*` names.
