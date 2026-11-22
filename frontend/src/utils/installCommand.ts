// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { InstallTarget } from '../types/enrollment'

/** What the install commands and manual steps are written from. */
export interface InstallParams {
  /** The server's own address, as the host reaches it. */
  server: string
  /** The enrollment token, shown once. */
  token: string
  /** The server's agent channel key, pinned by the host. */
  serverKey: string
}

export interface ManualStep {
  label: string
  text: string
}

const MAC_DIR = '/Library/Assimilate'
const WINDOWS_DIR = 'C:\\Program Files\\Assimilate'

/** The one-line command for `target`. */
export function installCommand(target: InstallTarget, p: InstallParams): string {
  const shell = `curl -fsSL ${p.server}/install.sh | sudo sh -s -- \\\n  --server ${p.server} \\\n  --token ${p.token} \\\n  --server-key ${p.serverKey}`
  const COMMANDS: Record<InstallTarget, string> = {
    linux: shell,
    macos: shell,
    windows: `& ([scriptblock]::Create((irm ${p.server}/install.ps1))) \`\n  -Server ${p.server} \`\n  -Token ${p.token} \`\n  -ServerKey ${p.serverKey}`,
  }
  return COMMANDS[target]
}

/**
 * The steps for a host that cannot pipe a script into a shell. The agent
 * trades the enrollment token itself with `--enroll`, so the long-lived token
 * never appears in these commands.
 */
export function manualSteps(target: InstallTarget, p: InstallParams): ManualStep[] {
  const enroll = `${p.server} --server-key ${p.serverKey}`
  const STEPS: Record<InstallTarget, ManualStep[]> = {
    linux: [
      {
        label: '1. Download and verify',
        text: `cd "$(mktemp -d)"\ncurl -fsSLo assimilate-agent "${p.server}/download/agent?os=linux&arch=$(uname -m)"\ncurl -fsSLo assimilate-agent.sha256 "${p.server}/download/agent.sha256?os=linux&arch=$(uname -m)"\nsha256sum -c assimilate-agent.sha256\nsudo install -m 755 assimilate-agent /usr/local/bin/assimilate-agent`,
      },
      {
        label: '2. Enroll and store the agent token where only root can read it',
        text: `sudo install -m 600 /dev/null /etc/assimilate-agent.env\nBORG_ENROLLMENT_TOKEN=${p.token} /usr/local/bin/assimilate-agent --enroll \\\n  --server-url ${enroll} \\\n  | sed 's/^/BORG_AGENT_TOKEN=/' | sudo tee /etc/assimilate-agent.env >/dev/null`,
      },
      {
        label: '3. Start on boot with systemd',
        text: `sudo tee /etc/systemd/system/assimilate-agent.service >/dev/null <<'EOF'\n[Unit]\nDescription=Assimilate backup agent\nAfter=network.target\n\n[Service]\nEnvironment=BORG_SERVER_URL=${p.server}\nEnvironment=BORG_SERVER_KEY=${p.serverKey}\nEnvironmentFile=/etc/assimilate-agent.env\nExecStart=/usr/local/bin/assimilate-agent\nRestart=on-failure\nRestartSec=10\n\n[Install]\nWantedBy=multi-user.target\nEOF\nsudo systemctl daemon-reload\nsudo systemctl enable --now assimilate-agent`,
      },
    ],
    macos: [
      {
        label: '1. Download the release and verify it',
        text: `# Pick the file for your Mac from the release the server was built from:\n#   assimilate-agent-<version>-aarch64-apple-darwin.tar.gz (Apple silicon)\n#   assimilate-agent-<version>-x86_64-apple-darwin.tar.gz (Intel)\n# https://github.com/alexmohr/assimilate/releases\nshasum -a 256 assimilate-agent-*-apple-darwin.tar.gz   # compare with SHA256SUMS`,
      },
      {
        label: '2. Extract into a folder only root can write',
        text: `sudo mkdir -p ${MAC_DIR}\nsudo tar -xzf assimilate-agent-*-apple-darwin.tar.gz --no-same-owner -C ${MAC_DIR}\nsudo xattr -dr com.apple.quarantine ${MAC_DIR}/assimilate-agent`,
      },
      {
        label: '3. Enroll and install the daemon, which starts at boot',
        text: `TOKEN=$(BORG_ENROLLMENT_TOKEN=${p.token} ${MAC_DIR}/assimilate-agent/assimilate-agent --enroll \\\n  --server-url ${enroll})\nsudo BORG_SERVER_URL=${p.server} \\\n  BORG_SERVER_KEY=${p.serverKey} \\\n  BORG_AGENT_TOKEN="$TOKEN" \\\n  ${MAC_DIR}/assimilate-agent/assimilate-agent service install`,
      },
    ],
    windows: [
      {
        label: '1. Download the release and verify it',
        text: `# Download assimilate-agent-<version>-x86_64-pc-windows-msvc.zip and SHA256SUMS from\n# https://github.com/alexmohr/assimilate/releases, then:\n(Get-FileHash .\\assimilate-agent-*-x86_64-pc-windows-msvc.zip).Hash   # compare with SHA256SUMS\nExpand-Archive .\\assimilate-agent-*-x86_64-pc-windows-msvc.zip "${WINDOWS_DIR}"`,
      },
      {
        label: '2. Enroll and install the service, which starts at boot',
        text: `$agent = "${WINDOWS_DIR}\\assimilate-agent\\assimilate-agent.exe"\n$env:BORG_ENROLLMENT_TOKEN = "${p.token}"\n$env:BORG_AGENT_TOKEN = & $agent --enroll --server-url ${enroll}\nRemove-Item Env:\\BORG_ENROLLMENT_TOKEN\n$env:BORG_SERVER_URL = "${p.server}"\n$env:BORG_SERVER_KEY = "${p.serverKey}"\n& $agent service install`,
      },
    ],
  }
  return STEPS[target]
}

/** What to say about starting the agent at boot, per platform. */
export const AUTOSTART_NOTES: Record<InstallTarget, string> = {
  linux:
    'systemd starts the agent at boot and restarts it if it stops. Check it with systemctl status assimilate-agent.',
  macos:
    'service install writes /Library/LaunchDaemons/com.assimilate.agent.plist, so launchd starts the agent at boot and again after every exit. Backing up folders such as ~/Library needs Full Disk Access for the agent and its borg (System Settings > Privacy & Security).',
  windows:
    'service install registers the assimilate-agent Windows service to start automatically at boot as LocalSystem. Manage it in Services (services.msc).',
}
