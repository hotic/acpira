import { spawn } from 'node:child_process';

// A system notification without a native module: each OS's own command line. Title and body never go through a shell or
// a script's source text: argv (osascript, notify-send) or environment variables (PowerShell) carry them
export interface NotifyCommand {
  command: string;
  args: string[];
  env?: Record<string, string>;
}

// Windows PowerShell's AppUserModelID: a toast needs a registered app id, and this one exists on every Windows 10 / 11
const POWERSHELL_AUMID = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe';

// A toast Windows will not show (notifications switched off globally or for PowerShell) is dropped silently while Show()
// still succeeds, so those switches are read first and a non-zero exit sends the caller to its in-window fallback
const WINDOWS_TOAST = [
  '$ErrorActionPreference = "Stop"',
  `if ((Get-ItemProperty "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\PushNotifications" -ErrorAction SilentlyContinue).ToastEnabled -eq 0) { exit 3 }`,
  `if ((Get-ItemProperty -LiteralPath "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings\\${POWERSHELL_AUMID}" -ErrorAction SilentlyContinue).Enabled -eq 0) { exit 3 }`,
  '[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null',
  '$x = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)',
  '$t = $x.GetElementsByTagName("text")',
  '$t.Item(0).AppendChild($x.CreateTextNode($env:ACPIRA_NOTIFY_TITLE)) > $null',
  '$t.Item(1).AppendChild($x.CreateTextNode($env:ACPIRA_NOTIFY_BODY)) > $null',
  `[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('${POWERSHELL_AUMID}').Show([Windows.UI.Notifications.ToastNotification]::new($x))`,
].join('; ');

export function notifyCommand(platform: NodeJS.Platform, title: string, body: string): NotifyCommand | undefined {
  switch (platform) {
    case 'darwin':
      // The notification is attributed to Script Editor; clicking it opens that app, not the window (no public API offers more)
      return { command: 'osascript', args: ['-e', 'on run argv', '-e', 'display notification (item 2 of argv) with title (item 1 of argv)', '-e', 'end run', title, body] };
    case 'win32':
      return {
        command: 'powershell.exe',
        args: ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', WINDOWS_TOAST],
        env: { ACPIRA_NOTIFY_TITLE: title, ACPIRA_NOTIFY_BODY: body },
      };
    case 'linux':
    case 'freebsd':
    case 'openbsd':
      return { command: 'notify-send', args: ['--app-name=Acpira', title, body] };
    default:
      return undefined;
  }
}

// Resolves false when the OS has no such command or it failed, so the caller can fall back to an in-window message
export function showSystemNotification(title: string, body: string): Promise<boolean> {
  const cmd = notifyCommand(process.platform, title, body);
  if (!cmd) return Promise.resolve(false);
  return new Promise(resolve => {
    try {
      const child = spawn(cmd.command, cmd.args, { env: { ...process.env, ...cmd.env }, stdio: 'ignore', windowsHide: true });
      child.on('error', () => resolve(false));
      child.on('exit', code => resolve(code === 0));
    } catch {
      resolve(false);
    }
  });
}
