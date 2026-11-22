// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { AUTOSTART_NOTES, installCommand, manualSteps, type InstallParams } from './installCommand'
import type { InstallTarget } from '../types/enrollment'

const PARAMS: InstallParams = {
  server: 'https://backup.example.com',
  token: 'enr_0123',
  serverKey: 'KEY=',
}

const TARGETS: readonly InstallTarget[] = ['linux', 'macos', 'windows']

describe('installCommand', () => {
  it('pipes the install script into a root shell on Linux and macOS', () => {
    for (const target of ['linux', 'macos'] as const) {
      const command = installCommand(target, PARAMS)
      expect(command).toContain('curl -fsSL https://backup.example.com/install.sh | sudo sh')
      expect(command).toContain('--server https://backup.example.com')
      expect(command).toContain('--token enr_0123')
      expect(command).toContain('--server-key KEY=')
    }
  })

  it('runs the PowerShell script with the same three values on Windows', () => {
    const command = installCommand('windows', PARAMS)

    expect(command).toContain('irm https://backup.example.com/install.ps1')
    expect(command).toContain('-Server https://backup.example.com')
    expect(command).toContain('-Token enr_0123')
    expect(command).toContain('-ServerKey KEY=')
  })
})

describe('manualSteps', () => {
  it('has the same three parts on every platform that downloads, enrolls and registers a service', () => {
    expect(manualSteps('linux', PARAMS)).toHaveLength(3)
    expect(manualSteps('macos', PARAMS)).toHaveLength(3)
    expect(manualSteps('windows', PARAMS)).toHaveLength(2)
  })

  it('downloads Linux from the server and checks its SHA-256', () => {
    const text = manualSteps('linux', PARAMS)
      .map((s) => s.text)
      .join('\n')

    expect(text).toContain('https://backup.example.com/download/agent?os=linux')
    expect(text).toContain('sha256sum -c')
    expect(text).toContain('systemctl enable --now assimilate-agent')
    expect(text).toContain('/etc/assimilate-agent.env')
  })

  it('starts macOS as a launchd daemon from a folder only root can write', () => {
    const text = manualSteps('macos', PARAMS)
      .map((s) => s.text)
      .join('\n')

    expect(text).toContain('/Library/Assimilate')
    expect(text).toContain('com.apple.quarantine')
    expect(text).toContain('service install')
  })

  it('registers the Windows service', () => {
    const text = manualSteps('windows', PARAMS)
      .map((s) => s.text)
      .join('\n')

    expect(text).toContain('service install')
    expect(text).toContain('Get-FileHash')
  })

  it('passes the enrollment token through the environment, not a flag', () => {
    for (const target of TARGETS) {
      const text = manualSteps(target, PARAMS)
        .map((s) => s.text)
        .join('\n')
      expect(text).toContain('BORG_ENROLLMENT_TOKEN')
      expect(text).not.toContain('--token')
      expect(text).toContain('--enroll')
    }
  })
})

describe('AUTOSTART_NOTES', () => {
  it('says what starts the agent at boot on each platform', () => {
    expect(AUTOSTART_NOTES.linux).toContain('systemd')
    expect(AUTOSTART_NOTES.macos).toContain('launchd')
    expect(AUTOSTART_NOTES.windows).toContain('Windows service')
  })
})
