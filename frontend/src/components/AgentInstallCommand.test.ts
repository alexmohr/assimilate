// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import AgentInstallCommand from './AgentInstallCommand.vue'

const { mockCopy } = vi.hoisted(() => ({ mockCopy: vi.fn() }))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({ copied: false, copy: mockCopy }),
}))

function generated(): ReturnType<typeof renderWithPlugins> {
  return renderWithPlugins(AgentInstallCommand, { props: { generated: true } })
}

async function pick(wrapper: ReturnType<typeof renderWithPlugins>, label: string): Promise<void> {
  const option = wrapper.findAll('[role="radio"]').find((b) => b.text() === label)
  await option?.trigger('click')
}

describe('AgentInstallCommand', () => {
  it('asks for expiry, hostname and tags before a command exists', () => {
    const wrapper = renderWithPlugins(AgentInstallCommand, { props: { generated: false } })

    expect(wrapper.find('#enroll-expiry').exists()).toBe(true)
    expect(wrapper.find('#enroll-hostname').exists()).toBe(true)
    expect(wrapper.find('#enroll-tags').exists()).toBe(true)
    expect(wrapper.find('[data-testid="install-command"]').exists()).toBe(false)
  })

  it('shows the curl one-liner on Linux and macOS', async () => {
    const wrapper = generated()
    const command = (): string => wrapper.get('[data-testid="install-command"]').text()

    expect(command()).toContain('install.sh | sudo sh')
    await pick(wrapper, 'macOS')
    expect(command()).toContain('install.sh | sudo sh')
  })

  it('shows the PowerShell equivalent on Windows', async () => {
    const wrapper = generated()
    await pick(wrapper, 'Windows')

    expect(wrapper.get('[data-testid="install-command"]').text()).toContain('install.ps1')
  })

  it('lists the autostart step for each platform', async () => {
    const wrapper = generated()
    const text = (): string => wrapper.text()

    expect(text()).toContain('systemctl enable --now assimilate-agent')
    await pick(wrapper, 'macOS')
    expect(text()).toContain('/Library/Assimilate')
    expect(text()).toContain('com.apple.quarantine')
    expect(text()).toContain('com.assimilate.agent.plist')
    await pick(wrapper, 'Windows')
    expect(text()).toContain('service install')
    expect(text()).toContain('services.msc')
  })

  it('copies a command from its copy button', async () => {
    const wrapper = generated()
    await wrapper.get('.token-box button').trigger('click')

    expect(mockCopy).toHaveBeenCalledWith(expect.stringContaining('install.sh'))
  })
})
