// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import RepoBorgConsole from './RepoBorgConsole.vue'

vi.mock('../api/client', () => ({ apiClient: { post: vi.fn() } }))

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoBorgConsole, { props: { repoId: 12, ...props } })
}

function ok(stdout = 'Repository: /backup/repos/server-daily', exit = 0) {
  return { data: { stdout, stderr: '', exit_code: exit } }
}

async function run(wrapper: ReturnType<typeof mount>, command: string): Promise<void> {
  await wrapper.find('input.console-input').setValue(command)
  await wrapper.find('button.btn-primary').trigger('click')
  await flushPromises()
}

describe('RepoBorgConsole', () => {
  beforeEach(() => {
    vi.mocked(apiClient.post)
      .mockReset()
      .mockResolvedValue(ok() as never)
  })

  it('will not run an empty command', async () => {
    const wrapper = mount()
    const button = wrapper.find('button.btn-primary')
    expect(button.attributes('disabled')).toBeDefined()

    await wrapper.find('input.console-input').setValue('   ')
    expect(button.attributes('disabled')).toBeDefined()

    await button.trigger('click')
    await flushPromises()
    expect(apiClient.post).not.toHaveBeenCalled()
  })

  // The server takes an argv array, not a command line, so the console has to
  // split the input rather than passing the raw string through.
  it('sends the command as argv against this repository', async () => {
    const wrapper = mount()
    await run(wrapper, 'list --short')
    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/exec', { args: ['list', '--short'] })
  })

  it('collapses runs of whitespace rather than sending empty arguments', async () => {
    const wrapper = mount()
    await run(wrapper, '  diff   ::a   ::b  ')
    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/exec', {
      args: ['diff', '::a', '::b'],
    })
  })

  it('runs against the repository it was given', async () => {
    const wrapper = mount({ repoId: 99 })
    await run(wrapper, 'info')
    expect(apiClient.post).toHaveBeenCalledWith('/repos/99/exec', { args: ['info'] })
  })

  it('runs on Enter as well as the button', async () => {
    const wrapper = mount()
    const input = wrapper.find('input.console-input')
    await input.setValue('info')
    await input.trigger('keydown.enter')
    await flushPromises()
    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/exec', { args: ['info'] })
  })

  it('fills the input from a suggested command', async () => {
    const wrapper = mount()
    const hints = wrapper.findAll('.console-hint-cmd')
    expect(hints.map((h) => h.text())).toContain('compact')

    await hints[0].trigger('click')
    expect((wrapper.find('input.console-input').element as HTMLInputElement).value).toBe('info')
  })

  describe('output', () => {
    it('shows stdout and the exit code', async () => {
      const wrapper = mount()
      await run(wrapper, 'info')
      expect(wrapper.find('.console-pre').text()).toContain('/backup/repos/server-daily')
      expect(wrapper.find('.console-output-header').text()).toContain('exit 0')
    })

    it('shows stderr in its own block', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { stdout: '', stderr: 'Repository is locked', exit_code: 2 },
      } as never)
      const wrapper = mount()
      await run(wrapper, 'check')
      expect(wrapper.find('.console-pre-stderr').text()).toBe('Repository is locked')
    })

    it('says so when the command printed nothing at all', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { stdout: '', stderr: '', exit_code: 0 },
      } as never)
      const wrapper = mount()
      await run(wrapper, 'compact')
      expect(wrapper.find('.console-empty').text()).toBe('(no output)')
    })

    // borg reserves exit 1 for warnings: a `check` that finds a repairable
    // inconsistency exits 1, and styling that as a hard failure would tell
    // the operator the wrong thing.
    it.each([
      [0, 'exit-ok'],
      [1, 'exit-warn'],
      [2, 'exit-err'],
      [130, 'exit-err'],
    ])('styles exit %i as %s', async (code, expected) => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { stdout: 'out', stderr: '', exit_code: code },
      } as never)
      const wrapper = mount()
      await run(wrapper, 'check')
      expect(wrapper.find(`.${expected}`).exists()).toBe(true)
    })

    it('clears the previous result before the next run', async () => {
      const wrapper = mount()
      await run(wrapper, 'info')
      expect(wrapper.find('.console-output').exists()).toBe(true)

      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('boom'))
      await run(wrapper, 'check')

      expect(wrapper.find('.console-output').exists()).toBe(false)
    })
  })

  it('reports a failed request as an error rather than as output', async () => {
    vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('exec refused'))
    const wrapper = mount()
    await run(wrapper, 'delete')

    expect(wrapper.find('.console-error').exists()).toBe(true)
    expect(wrapper.find('.console-output').exists()).toBe(false)
  })

  it('clears a previous error once a run succeeds', async () => {
    vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('exec refused'))
    const wrapper = mount()
    await run(wrapper, 'delete')
    expect(wrapper.find('.console-error').exists()).toBe(true)

    await run(wrapper, 'info')
    expect(wrapper.find('.console-error').exists()).toBe(false)
  })
})
